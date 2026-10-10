//! The actions on a physical device: loading the app, deleting it and setting the device up.
//! Linux runs the tools in the container, which reaches USB. macOS and Windows run them on the host.

use crate::app::App;
use crate::command::Command;
use crate::device::Device;
use crate::host_python::HostPython;
use crate::platform::Platform;
use crate::task::{InContainer, Options, PlanError, Step, Task, Warning};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTask {
    /// Installs the built app.
    Load,
    Delete,
    /// Sets up a new device with the PIN and seed of the settings.
    QuickSetup,
}

/// The settings that the actions on a physical device depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSettings {
    /// The key to open the secure channel with, such as the host's `SCP_PRIVKEY`. A random one
    /// is used when there is none.
    pub scp_private_key: Option<String>,
    pub onboarding: Onboarding,
    /// Actions on a Nano X need a special development device, so they are off unless allowed.
    pub allow_nano_x: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub pin: String,
    pub seed: String,
}

impl Onboarding {
    /// The seed the settings start with. Everyone knows it.
    pub const DEFAULT_SEED: &str = "glory promote mansion idle axis finger extra february uncover one trip resource lawn turtle enact monster seven myth punch hobby comfort wild raise skin";

    pub fn uses_default_seed(&self) -> bool {
        self.seed == Self::DEFAULT_SEED
    }
}

/// Whether the settings allow actions on a physical device of this model.
pub fn allowed(device: Device, settings: &DeviceSettings) -> bool {
    device != Device::NanoX || settings.allow_nano_x
}

pub(crate) fn warnings(task: Task, options: &Options) -> Vec<Warning> {
    let default_seed = options
        .device
        .as_ref()
        .is_some_and(|settings| settings.onboarding.uses_default_seed());
    if task == Task::Device(DeviceTask::QuickSetup) && default_seed {
        vec![Warning::DefaultOnboardingSeed]
    } else {
        Vec::new()
    }
}

pub(crate) fn steps(
    task: DeviceTask,
    app: &App,
    device: Device,
    settings: &DeviceSettings,
    host: &HostPython,
) -> Result<Vec<Step>, PlanError> {
    if !allowed(device, settings) {
        return Err(PlanError::DeviceActionsOff(device));
    }
    let call = match task {
        DeviceTask::Load => load(app, device, settings, host.platform),
        DeviceTask::Delete => delete(app, device, settings)?,
        DeviceTask::QuickSetup => quick_setup(settings),
    };
    Ok(call.steps(device, host))
}

/// A tool of `ledgerblue`, the Python package that talks to the device.
struct Call {
    module: &'static str,
    args: Arguments,
    /// The tool wants the SDK of the device in `BOLOS_SDK`.
    needs_sdk: bool,
}

/// Arguments to a program, and the same arguments as they may be shown.
#[derive(Default)]
struct Arguments {
    real: Vec<String>,
    shown: Vec<String>,
}

impl Arguments {
    fn arg(mut self, arg: impl Into<String>) -> Self {
        let arg = arg.into();
        self.shown.push(arg.clone());
        self.real.push(arg);
        self
    }

    fn args<I, S>(self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        args.into_iter().fold(self, Self::arg)
    }

    /// An argument that must never be shown.
    fn secret(mut self, arg: impl Into<String>) -> Self {
        self.real.push(arg.into());
        self.shown.push("REDACTED".to_owned());
        self
    }
}

fn load(app: &App, device: Device, settings: &DeviceSettings, platform: Platform) -> Call {
    let file = |name: &str| {
        let in_app = app.in_build_directory(&format!(
            "build/{}/bin/{name}",
            device.spec().build_dir_name
        ));
        match platform {
            Platform::Linux => in_app,
            Platform::MacOs | Platform::Windows => app
                .folder
                .join(in_app.trim_start_matches("./"))
                .to_string_lossy()
                .into_owned(),
        }
    };
    Call {
        module: "ledgerblue.runScript",
        args: secure_channel(settings).arg("--scp").args([
            "--fileName",
            &file("app.apdu"),
            "--elfFile",
            &file("app.elf"),
        ]),
        needs_sdk: false,
    }
}

fn delete(app: &App, device: Device, settings: &DeviceSettings) -> Result<Call, PlanError> {
    let name = app.name().ok_or(PlanError::UnknownAppName)?;
    Ok(Call {
        module: "ledgerblue.deleteApp",
        args: secure_channel(settings).args([
            "--targetId",
            &format!("0x{:08x}", device.spec().target_id),
            "--appName",
            name,
        ]),
        needs_sdk: false,
    })
}

fn quick_setup(settings: &DeviceSettings) -> Call {
    let onboarding = &settings.onboarding;
    Call {
        module: "ledgerblue.hostOnboard",
        args: Arguments::default()
            .args(["--apdu", "--id", "0"])
            .arg("--pin")
            .secret(&onboarding.pin)
            .args(["--prefix", "", "--passphrase", ""])
            .arg("--words")
            .secret(&onboarding.seed),
        needs_sdk: true,
    }
}

fn secure_channel(settings: &DeviceSettings) -> Arguments {
    match &settings.scp_private_key {
        Some(key) => Arguments::default().arg("--rootPrivateKey").secret(key),
        None => Arguments::default(),
    }
}

impl Call {
    fn steps(self, device: Device, host: &HostPython) -> Vec<Step> {
        let label = shell_words::join(
            [host.system_python(), "-m", self.module]
                .into_iter()
                .chain(self.args.shown.iter().map(String::as_str)),
        );

        if host.platform == Platform::Linux {
            let argv = ["python3", "-m", self.module]
                .map(str::to_owned)
                .into_iter()
                .chain(self.args.real);
            let step = InContainer::new(argv).label(label);
            return vec![
                if self.needs_sdk {
                    step.sdk(device)
                } else {
                    step
                }
                .into(),
            ];
        }

        let run = Command::new(host.python())
            .args(["-m", self.module])
            .args(self.args.real)
            .label(label);
        let mut steps = host.environment(&["ledgerblue"]);
        steps.push(Step::on_host(run));
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_app::MakefileInfo;
    use crate::docker::{ContainerUser, RunId};
    use crate::language::Language;
    use crate::rust_app::RustAppPackage;
    use crate::target::Target;
    use crate::task::{Plan, plan_task};

    const VENV: &str = "/home/u/venvs/app-x";

    fn settings() -> DeviceSettings {
        DeviceSettings {
            scp_private_key: None,
            onboarding: Onboarding {
                pin: "1234".to_owned(),
                seed: "one two three".to_owned(),
            },
            allow_nano_x: false,
        }
    }

    fn options(settings: DeviceSettings, platform: Platform) -> Options {
        Options {
            device: Some(settings),
            host: Some(HostPython {
                platform,
                venv: VENV.into(),
            }),
            ..Options::default()
        }
    }

    fn linux() -> Options {
        options(settings(), Platform::Linux)
    }

    fn app(language: Language, name: Option<&str>) -> App {
        App {
            folder: "/work/app-x".into(),
            language,
            build_directory: "./app".to_owned(),
            devices: Device::ALL.to_vec(),
            rust: (language == Language::Rust).then(|| RustAppPackage {
                package_name: "app-x-rust".to_owned(),
                app_name: name.unwrap_or_default().to_owned(),
            }),
            build_use_cases: Vec::new(),
            test_suites: Vec::new(),
            makefile: (language == Language::C)
                .then(|| {
                    name.map(|name| MakefileInfo {
                        name: name.to_owned(),
                        variants: None,
                    })
                })
                .flatten(),
            compose_file: None,
        }
    }

    fn c_app() -> App {
        app(Language::C, Some("X"))
    }

    fn run(
        task: DeviceTask,
        app: &App,
        device: Device,
        options: &Options,
    ) -> Result<Plan, PlanError> {
        plan_task(Task::Device(task), app, &Target::one(device), options)
    }

    fn only_job_steps(plan: &Plan) -> &[Step] {
        assert_eq!(plan.jobs.len(), 1, "{plan:?}");
        &plan.jobs[0].steps
    }

    fn in_container(step: &Step) -> &InContainer {
        match step {
            Step::InContainer(step) => step,
            Step::OnHost(host) => panic!("expected a step in a container: {}", host.command),
        }
    }

    fn on_host(step: &Step) -> &Command {
        match step {
            Step::OnHost(host) => &host.command,
            Step::InContainer(step) => panic!("expected a step on the host: {:?}", step.argv),
        }
    }

    fn shown(step: &InContainer) -> String {
        let run_id = RunId::new("run1").unwrap();
        step.command("c", ContainerUser::Root, &run_id).to_string()
    }

    #[test]
    fn loading_on_linux_runs_ledgerblue_in_the_container() {
        let plan = run(DeviceTask::Load, &c_app(), Device::Stax, &linux()).unwrap();

        assert_eq!(plan.jobs[0].device, Device::Stax);
        assert!(plan.jobs[0].uses_container());
        let step = in_container(&only_job_steps(&plan)[0]);
        assert_eq!(
            step.argv,
            [
                "python3",
                "-m",
                "ledgerblue.runScript",
                "--scp",
                "--fileName",
                "./app/build/stax/bin/app.apdu",
                "--elfFile",
                "./app/build/stax/bin/app.elf",
            ]
        );
        assert_eq!(step.sdk, None);
    }

    #[test]
    fn the_secure_channel_key_is_passed_but_never_shown() {
        let settings = DeviceSettings {
            scp_private_key: Some("c0ffee".to_owned()),
            ..settings()
        };
        let options = options(settings, Platform::Linux);

        let plan = run(DeviceTask::Load, &c_app(), Device::Stax, &options).unwrap();

        let step = in_container(&only_job_steps(&plan)[0]);
        assert_eq!(
            step.argv[3..6],
            ["--rootPrivateKey", "c0ffee", "--scp"],
            "{:?}",
            step.argv
        );
        let shown = shown(step);
        assert!(!shown.contains("c0ffee"), "{shown}");
        assert!(shown.contains("--rootPrivateKey REDACTED"), "{shown}");
    }

    #[test]
    fn deleting_names_the_app_and_the_target_id_of_the_device() {
        let rust = app(Language::Rust, Some("My App"));

        let plan = run(DeviceTask::Delete, &rust, Device::NanoSPlus, &linux()).unwrap();

        let step = in_container(&only_job_steps(&plan)[0]);
        assert_eq!(
            step.argv,
            [
                "python3",
                "-m",
                "ledgerblue.deleteApp",
                "--targetId",
                "0x33100004",
                "--appName",
                "My App",
            ]
        );
    }

    #[test]
    fn deleting_a_c_app_needs_its_name_from_the_makefile() {
        let unnamed = app(Language::C, None);

        let result = run(DeviceTask::Delete, &unnamed, Device::Stax, &linux());

        assert_eq!(result.unwrap_err(), PlanError::UnknownAppName);
    }

    #[test]
    fn quick_setup_gives_the_pin_and_the_seed_and_shows_neither() {
        let plan = run(DeviceTask::QuickSetup, &c_app(), Device::Flex, &linux()).unwrap();

        let step = in_container(&only_job_steps(&plan)[0]);
        assert_eq!(
            step.argv[2..],
            [
                "ledgerblue.hostOnboard",
                "--apdu",
                "--id",
                "0",
                "--pin",
                "1234",
                "--prefix",
                "",
                "--passphrase",
                "",
                "--words",
                "one two three",
            ]
        );
        assert_eq!(step.sdk, Some(Device::Flex));
        let shown = shown(step);
        assert!(
            !shown.contains("1234") && !shown.contains("one two"),
            "{shown}"
        );
    }

    #[test]
    fn the_default_seed_comes_with_a_warning() {
        let default_seed = options(
            DeviceSettings {
                onboarding: Onboarding {
                    pin: "1234".to_owned(),
                    seed: Onboarding::DEFAULT_SEED.to_owned(),
                },
                ..settings()
            },
            Platform::Linux,
        );

        let with_default = run(
            DeviceTask::QuickSetup,
            &c_app(),
            Device::Flex,
            &default_seed,
        );
        let with_own = run(DeviceTask::QuickSetup, &c_app(), Device::Flex, &linux());
        let load = run(DeviceTask::Load, &c_app(), Device::Flex, &default_seed);

        assert_eq!(
            with_default.unwrap().warnings,
            [Warning::DefaultOnboardingSeed]
        );
        assert!(with_own.unwrap().warnings.is_empty());
        assert!(load.unwrap().warnings.is_empty());
    }

    #[test]
    fn a_nano_x_needs_the_setting_that_allows_it() {
        let off = linux();
        let on = options(
            DeviceSettings {
                allow_nano_x: true,
                ..settings()
            },
            Platform::Linux,
        );

        for task in [DeviceTask::Load, DeviceTask::Delete, DeviceTask::QuickSetup] {
            assert_eq!(
                run(task, &c_app(), Device::NanoX, &off).unwrap_err(),
                PlanError::DeviceActionsOff(Device::NanoX)
            );
            assert!(run(task, &c_app(), Device::NanoX, &on).is_ok());
            assert!(run(task, &c_app(), Device::Stax, &off).is_ok());
        }
    }

    #[test]
    fn only_one_device_is_targeted_and_the_settings_and_host_are_needed() {
        let two = Target::new([Device::Stax, Device::Flex]).unwrap();
        let task = Task::Device(DeviceTask::Load);
        let stax = Target::one(Device::Stax);
        let without_settings = Options {
            device: None,
            ..linux()
        };
        let without_host = Options {
            host: None,
            ..linux()
        };

        let several = plan_task(task, &c_app(), &two, &linux());
        let unset = plan_task(task, &c_app(), &stax, &without_settings);
        let no_host = plan_task(task, &c_app(), &stax, &without_host);

        assert_eq!(several.unwrap_err(), PlanError::NeedsOneDevice(task));
        assert_eq!(unset.unwrap_err(), PlanError::NoDeviceSettings);
        assert_eq!(no_host.unwrap_err(), PlanError::NoHostPython);
    }

    #[test]
    fn on_macos_the_tools_run_on_the_host_in_their_own_python_environment() {
        let options = options(settings(), Platform::MacOs);

        let plan = run(DeviceTask::Load, &c_app(), Device::Stax, &options).unwrap();

        assert!(!plan.jobs[0].uses_container());
        let steps = only_job_steps(&plan);
        assert_eq!(steps.len(), 3);
        let python = format!("{VENV}/bin/python");
        assert_eq!(
            *on_host(&steps[0]),
            Command::new("python3").args(["-m", "venv", VENV])
        );
        assert_eq!(
            *on_host(&steps[1]),
            Command::new(python.as_str()).args([
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "ledgerblue"
            ])
        );
        let load = on_host(&steps[2]);
        assert_eq!(load.program, python);
        assert_eq!(
            load.args,
            [
                "-m",
                "ledgerblue.runScript",
                "--scp",
                "--fileName",
                "/work/app-x/app/build/stax/bin/app.apdu",
                "--elfFile",
                "/work/app-x/app/build/stax/bin/app.elf",
            ]
        );
    }

    #[test]
    fn on_windows_the_tools_use_the_python_of_the_environment() {
        let options = options(settings(), Platform::Windows);

        let plan = run(DeviceTask::Delete, &c_app(), Device::Stax, &options).unwrap();

        let steps = only_job_steps(&plan);
        assert_eq!(on_host(&steps[0]).program, "python");
        let host = options.host.unwrap();
        assert_eq!(on_host(&steps[2]).program, host.python());
    }

    #[test]
    fn on_the_host_the_secrets_are_given_too_and_not_shown() {
        let options = options(settings(), Platform::MacOs);

        let plan = run(DeviceTask::QuickSetup, &c_app(), Device::Stax, &options).unwrap();

        let setup = on_host(&only_job_steps(&plan)[2]);
        assert!(setup.args.contains(&"1234".to_owned()));
        let shown = setup.to_string();
        assert!(
            !shown.contains("1234") && !shown.contains("one two"),
            "{shown}"
        );
        assert!(
            shown.starts_with("python3 -m ledgerblue.hostOnboard"),
            "{shown}"
        );
    }
}
