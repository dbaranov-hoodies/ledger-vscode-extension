//! What each action of the extension runs, as commands for the containers of a target.
//!
//! A [`Plan`] has one [`Job`] per device. The jobs are independent and run side by side, each for
//! its device, and the steps of a job run one after the other.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::app::{App, join_path};
use crate::command::Command;
use crate::device::Device;
use crate::device_ops::{self, DeviceSettings, DeviceTask};
use crate::docker::{self, APP_DIR, ContainerUser, RunId};
use crate::failures;
use crate::host_python::{HostPython, path_text};
use crate::language::Language;
use crate::manifest::{BuildUseCase, UseCaseOrigin};
use crate::target::Target;
use crate::test_deps::{self, TestDependencies};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    BuildApp,
    CleanAndBuild,
    CleanTargetBuild,
    CleanAllBuilds,
    UpdateRustSdk,
    RunEmulator,
    KillEmulator,
    RunTests(TestMode),
    EnforcerChecks,
    OpenTerminal,
    /// On a physical device, which only one device of the target can be.
    Device(DeviceTask),
    /// A terminal in the service of the app's `docker-compose.yml`.
    ComposeTerminal,
    /// Clones and builds the apps the tests of a suite need, with the scripts of the suite.
    /// `clean` throws away what was prepared before.
    PrepareTestDependencies {
        clean: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestMode {
    Headless,
    WithDisplay,
    /// On a physical device: only one can be plugged in, so only one device can be targeted.
    OnDevice,
    /// Regenerates the reference screenshots.
    GoldenRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub name: String,
    pub value: String,
}

/// Which tests to run, and where they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRun {
    /// The directory of the test suite, relative to the app folder.
    pub directory: String,
    /// The tests to run, as paths relative to the app folder. All the tests when empty.
    pub paths: Vec<String>,
}

/// How to run the compose file of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeRun {
    /// One of the services that `docker compose config --services` lists.
    pub service: String,
    pub user: ContainerUser,
}

/// What the user chose, beyond the target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub build_use_case: Option<BuildUseCase>,
    /// The variant of a C app.
    pub variant: Option<Variant>,
    pub verbose_tests: bool,
    /// One guideline check to run, or all of them.
    pub enforcer_check: Option<String>,
    pub tests: Option<TestRun>,
    /// A shell command to run before the tests, to install what they need.
    pub test_requirements: Option<String>,
    /// Needed by the actions on a physical device.
    pub device: Option<DeviceSettings>,
    /// The Python environment of the host, for what runs there.
    pub host: Option<HostPython>,
    pub compose: Option<ComposeRun>,
    pub test_dependencies: Option<TestDependencies>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error("{0:?} is not available for {1:?} apps")]
    NotAvailable(Task, Language),
    #[error("{0:?} needs exactly one device")]
    NeedsOneDevice(Task),
    #[error("a Rust app needs the names of its Cargo.toml to be built")]
    MissingRustPackage,
    #[error("the app has no functional tests")]
    NoTests,
    #[error("the settings of the actions on a physical device are missing")]
    NoDeviceSettings,
    #[error("actions on a {0:?} are turned off in the settings")]
    DeviceActionsOff(Device),
    #[error("the name of the app is not known yet")]
    UnknownAppName,
    #[error("this action runs on the host, and its Python environment is missing")]
    NoHostPython,
    #[error("the app has no docker-compose.yml")]
    NoComposeFile,
    #[error("the service of the compose file to run is missing")]
    NoComposeService,
    #[error("the test suite to prepare is missing")]
    NoTestDependencies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub jobs: Vec<Job>,
    pub warnings: Vec<Warning>,
}

/// Something to tell the user before the plan runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// The device is set up with a seed that everyone knows.
    DefaultOnboardingSeed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub device: Device,
    /// Run in order. The job stops at the first step that fails, unless that step keeps going.
    pub steps: Vec<Step>,
}

impl Job {
    /// Whether any step runs in the container of the device.
    pub fn uses_container(&self) -> bool {
        self.steps
            .iter()
            .any(|step| matches!(step, Step::InContainer(_)))
    }
}

/// One thing to run for a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    InContainer(InContainer),
    OnHost(OnHost),
}

impl Step {
    pub fn on_host(command: Command) -> Self {
        Self::OnHost(OnHost::new(command))
    }

    pub fn continue_on_error(&self) -> bool {
        match self {
            Self::InContainer(step) => step.continue_on_error,
            Self::OnHost(_) => false,
        }
    }
}

/// A command to run on the machine the extension runs on, as Git and a physical device need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnHost {
    pub command: Command,
    /// The command is typed into, as a shell is.
    pub tty: bool,
}

impl OnHost {
    pub fn new(command: Command) -> Self {
        Self {
            command,
            tty: false,
        }
    }

    pub fn tty(mut self) -> Self {
        self.tty = true;
        self
    }
}

impl From<InContainer> for Step {
    fn from(step: InContainer) -> Self {
        Self::InContainer(step)
    }
}

/// Runs `bash` with these arguments: the SDK to use, the directory to go to, whether to use the
/// Python environment, and then the program. Everything is an argument, nothing is pasted into
/// the script.
const WRAPPER: &str = r##"
sdk="$1"; dir="$2"; venv="$3"; shift 3
if [ -n "$sdk" ]; then export BOLOS_SDK="${!sdk}"; fi
if [ "$venv" = 1 ]; then export VIRTUAL_ENV=/opt/venv PATH="/opt/venv/bin:$PATH"; fi
cd "$dir" || exit 1
exec "$@"
"##;

/// A shell function that installs the requirements of a suite, if it has any. The containers of
/// an app share one pip cache, so they install one at a time: the first fills the cache, and
/// the others find everything in it instead of downloading and building it all again.
macro_rules! install_requirements {
    () => {
        r##"
install_requirements() {
  [ -f "$1" ] || return 0
  cache="${PIP_CACHE_DIR:-/pip-cache}"
  if [ -w "$cache" ] && command -v flock > /dev/null; then
    (
      flock -n 9 || { echo "Waiting for another container to install the requirements..."; flock 9; }
      pip install -r "$1"
    ) 9> "$cache/.install.lock"
  else
    pip install -r "$1"
  fi
}
"##
    };
}

/// Installs the requirements file given as the first argument, see [`install_requirements!`].
const INSTALL_REQUIREMENTS_SCRIPT: &str =
    concat!(install_requirements!(), r#"install_requirements "$1""#);

/// Lists the tests of a suite with `pytest --collect-only`, once the requirements of the suite
/// are installed. The option that gives the device is `--device` or the older `--model`, and
/// pytest exits with 5 when there is no test, which is no failure.
const COLLECT_SCRIPT: &str = concat!(
    install_requirements!(),
    r##"
dir="$1"; device="$2"
install_requirements "$dir/requirements.txt" > /dev/null 2>&1
cd "$dir" || exit 1
option=$(pytest --help | awk '/[Cc]ustom options/,/^$/' | grep -E -- '--model|--device' | head -n 1 | tr ' =' '\n' | grep -v '^$' | head -n 1)
case "$option" in
  --device) pytest --collect-only -q --device "$device" ;;
  --model) pytest --collect-only -q --model "$device" ;;
  *) pytest --collect-only -q ;;
esac
status=$?
[ "$status" -eq 5 ] && exit 0
exit "$status"
"##
);

/// Lists the tests of the suite in `directory`, relative to the app folder, as pytest sees them
/// for this device. What it prints is read with `TestTree::from_collect_output`.
pub fn collect_tests(directory: &str, device: Device) -> InContainer {
    InContainer::new([
        "bash",
        "-c",
        COLLECT_SCRIPT,
        "_",
        directory,
        device.spec().name,
    ])
    .venv()
}

/// A program to run inside a container, without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InContainer {
    pub argv: Vec<String>,
    /// The device whose SDK the program needs: `BOLOS_SDK` is set to it.
    pub sdk: Option<Device>,
    /// The directory to run in, relative to the app folder.
    pub dir: Option<String>,
    /// Use the Python environment of the image.
    pub venv: bool,
    /// The program is typed into, as a shell is.
    pub tty: bool,
    /// The job goes on when this step fails.
    pub continue_on_error: bool,
    /// What to show instead of the program and its arguments, when they hold a secret.
    pub label: Option<String>,
}

impl InContainer {
    pub fn new<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let argv: Vec<String> = argv.into_iter().map(Into::into).collect();
        assert!(!argv.is_empty(), "a step runs a program");
        Self {
            argv,
            sdk: None,
            dir: None,
            venv: false,
            tty: false,
            continue_on_error: false,
            label: None,
        }
    }

    pub fn sdk(mut self, device: Device) -> Self {
        self.sdk = Some(device);
        self
    }

    pub fn in_dir(mut self, dir: &str) -> Self {
        self.dir = Some(dir.to_owned());
        self
    }

    pub fn venv(mut self) -> Self {
        self.venv = true;
        self
    }

    pub fn tty(mut self) -> Self {
        self.tty = true;
        self
    }

    pub fn keep_going(mut self) -> Self {
        self.continue_on_error = true;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The `docker exec` that runs this step, its processes tagged with the run id.
    pub fn command(&self, container: &str, user: ContainerUser, run_id: &RunId) -> Command {
        let sdk = self
            .sdk
            .map(|device| device.sdk_env_var())
            .unwrap_or_default();
        let dir = self.dir.clone().unwrap_or_else(|| ".".to_owned());
        let venv = if self.venv { "1" } else { "0" };

        let mut script: Vec<String> = ["bash", "-c", WRAPPER, "_"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        script.extend([sdk, dir, venv.to_owned()]);
        script.extend(self.argv.iter().cloned());
        let label = match &self.label {
            Some(label) => label.clone(),
            None => shell_words::join(&self.argv),
        };
        docker::exec_tagged(container, user, run_id, self.tty, &script).label(label)
    }
}

/// Turns an action into the commands to run for a target.
pub fn plan_task(
    task: Task,
    app: &App,
    target: &Target,
    options: &Options,
) -> Result<Plan, PlanError> {
    let devices: Vec<Device> = target.devices().iter().copied().collect();
    let jobs = match task {
        Task::BuildApp => per_device(&devices, |device| build(app, device, options))?,
        Task::CleanAndBuild => {
            require(task, app, Language::C)?;
            per_device(&devices, |device| {
                Ok(vec![
                    make(app, Some(device), ["clean_target"]),
                    c_build(app, device, options),
                ])
            })?
        }
        Task::CleanTargetBuild => {
            require(task, app, Language::C)?;
            per_device(&devices, |device| {
                Ok(vec![make(app, Some(device), ["clean_target"])])
            })?
        }
        Task::CleanAllBuilds => run_once(&devices, clean_all(app)),
        Task::UpdateRustSdk => {
            require(task, app, Language::Rust)?;
            let update = InContainer::new(["cargo", "update", "ledger_device_sdk"])
                .in_dir(&app.build_directory);
            run_once(&devices, vec![update])
        }
        Task::RunEmulator => per_device(&devices, |device| Ok(vec![run_emulator(app, device)]))?,
        Task::KillEmulator => per_device(&devices, |_| {
            Ok(vec![InContainer::new(["pkill", "-f", "speculos"])])
        })?,
        Task::RunTests(TestMode::OnDevice) => single_device(task, target, |device| {
            tests(device, TestMode::OnDevice, options)
        })?,
        Task::RunTests(mode) => per_device(&devices, |device| tests(device, mode, options))?,
        Task::EnforcerChecks => enforcer(app, &devices, target, options),
        Task::OpenTerminal => per_device(&devices, |device| {
            Ok(vec![InContainer::new(["bash"]).sdk(device).tty()])
        })?,
        Task::Device(device_task) => {
            let settings = options.device.as_ref().ok_or(PlanError::NoDeviceSettings)?;
            let host = options.host.as_ref().ok_or(PlanError::NoHostPython)?;
            single_device(task, target, |device| {
                device_ops::steps(device_task, app, device, settings, host)
            })?
        }
        Task::ComposeTerminal => {
            let file = app.compose_file.as_ref().ok_or(PlanError::NoComposeFile)?;
            let run = options
                .compose
                .as_ref()
                .ok_or(PlanError::NoComposeService)?;
            let file = path_text(&app.folder.join(file));
            let command = docker::compose_run(&file, run.user, &run.service);
            run_once(&devices, vec![Step::OnHost(OnHost::new(command).tty())])
        }
        Task::PrepareTestDependencies { clean } => {
            let dependencies = options
                .test_dependencies
                .as_ref()
                .ok_or(PlanError::NoTestDependencies)?;
            let host = options.host.as_ref().ok_or(PlanError::NoHostPython)?;
            run_once(&devices, test_deps::steps(dependencies, clean, app, host))
        }
    };
    Ok(Plan {
        jobs,
        warnings: device_ops::warnings(task, options),
    })
}

fn require(task: Task, app: &App, language: Language) -> Result<(), PlanError> {
    if app.language == language {
        Ok(())
    } else {
        Err(PlanError::NotAvailable(task, app.language))
    }
}

fn per_device<S: Into<Step>>(
    devices: &[Device],
    steps: impl Fn(Device) -> Result<Vec<S>, PlanError>,
) -> Result<Vec<Job>, PlanError> {
    devices
        .iter()
        .map(|&device| {
            Ok(Job {
                device,
                steps: steps(device)?.into_iter().map(Into::into).collect(),
            })
        })
        .collect()
}

/// For actions that are not about a device: run once, for the first device.
fn run_once<S: Into<Step>>(devices: &[Device], steps: Vec<S>) -> Vec<Job> {
    vec![Job {
        device: devices[0],
        steps: steps.into_iter().map(Into::into).collect(),
    }]
}

fn single_device<S: Into<Step>>(
    task: Task,
    target: &Target,
    steps: impl FnOnce(Device) -> Result<Vec<S>, PlanError>,
) -> Result<Vec<Job>, PlanError> {
    let device = target.single().ok_or(PlanError::NeedsOneDevice(task))?;
    Ok(vec![Job {
        device,
        steps: steps(device)?.into_iter().map(Into::into).collect(),
    }])
}

fn make<'a>(
    app: &App,
    sdk: Option<Device>,
    goal: impl IntoIterator<Item = &'a str>,
) -> InContainer {
    let mut argv = vec![
        "make".to_owned(),
        "-C".to_owned(),
        app.build_directory.clone(),
    ];
    argv.extend(goal.into_iter().map(str::to_owned));
    let step = InContainer::new(argv);
    match sdk {
        Some(device) => step.sdk(device),
        None => step,
    }
}

fn build(app: &App, device: Device, options: &Options) -> Result<Vec<InContainer>, PlanError> {
    match app.language {
        Language::C => Ok(vec![c_build(app, device, options)]),
        Language::Rust => rust_build(app, device, options),
    }
}

/// `release` builds with no option. A use case defined by the manifest is a Makefile target, and
/// the default `debug` is a flag.
fn c_build_options(options: &Options) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(use_case) = &options.build_use_case {
        match (use_case.name.as_str(), use_case.origin) {
            ("release", _) => {}
            ("debug", UseCaseOrigin::BuiltIn) => args.push("DEBUG=1".to_owned()),
            (name, _) => args.push(name.to_owned()),
        }
    }
    if let Some(variant) = &options.variant {
        args.push(format!("{}={}", variant.name, variant.value));
    }
    args
}

fn c_build(app: &App, device: Device, options: &Options) -> InContainer {
    let options = c_build_options(options);
    let goal = std::iter::once("-j").chain(options.iter().map(String::as_str));
    make(app, Some(device), goal)
}

fn rust_build(app: &App, device: Device, options: &Options) -> Result<Vec<InContainer>, PlanError> {
    let package = app.rust.as_ref().ok_or(PlanError::MissingRustPackage)?;
    let bin = format!("build/{}/bin", device.spec().build_dir_name);
    let debug = options
        .build_use_case
        .as_ref()
        .is_some_and(|use_case| use_case.name == "debug");

    let mut cargo = vec![
        "cargo".to_owned(),
        "ledger".to_owned(),
        "build".to_owned(),
        device.sdk_model(Language::Rust).to_owned(),
        "--".to_owned(),
    ];
    if debug {
        cargo.extend(["--features".to_owned(), "debug".to_owned()]);
    }
    cargo.extend([
        "-Zunstable-options".to_owned(),
        format!("--artifact-dir={bin}"),
    ]);

    let in_build_directory =
        |argv: Vec<String>| InContainer::new(argv).in_dir(&app.build_directory);
    let rename = |from: String, to: &str| {
        in_build_directory(vec!["mv".to_owned(), from, format!("{bin}/{to}")])
    };
    Ok(vec![
        in_build_directory(cargo),
        rename(format!("{bin}/{}", package.package_name), "app.elf"),
        rename(format!("{bin}/{}.apdu", package.package_name), "app.apdu"),
    ])
}

fn clean_all(app: &App) -> Vec<InContainer> {
    match app.language {
        Language::C => vec![make(app, None, ["clean"])],
        Language::Rust => vec![
            InContainer::new(["cargo", "clean"]).in_dir(&app.build_directory),
            InContainer::new(["rm", "-rf", "build"]).in_dir(&app.build_directory),
        ],
    }
}

fn run_emulator(app: &App, device: Device) -> InContainer {
    let spec = device.spec();
    let elf = app.in_build_directory(&format!("build/{}/bin/app.elf", spec.build_dir_name));
    InContainer::new(["speculos", "--model", spec.name, &elf])
}

fn tests(device: Device, mode: TestMode, options: &Options) -> Result<Vec<InContainer>, PlanError> {
    let run = options.tests.as_ref().ok_or(PlanError::NoTests)?;

    let mut steps = Vec::new();
    if let Some(requirements) = &options.test_requirements {
        steps.push(
            InContainer::new(["bash", "-c", requirements])
                .venv()
                .keep_going(),
        );
    }
    let requirements_file = join_path(&run.directory, "requirements.txt");
    steps.push(
        InContainer::new([
            "bash",
            "-c",
            INSTALL_REQUIREMENTS_SCRIPT,
            "_",
            &requirements_file,
        ])
        .venv()
        .keep_going()
        .label(format!("pip install -r {requirements_file}")),
    );

    let mut pytest = vec!["pytest".to_owned()];
    if run.paths.is_empty() {
        pytest.push(run.directory.clone());
    } else {
        pytest.extend(run.paths.iter().cloned());
    }
    pytest.extend(["--color=yes", "--tb=short", "-v"].map(str::to_owned));
    // A cache for each device, so that the failures of every device are known after parallel
    // runs and are not lost when another device runs.
    let cache = format!("{APP_DIR}/{}", failures::cache_dir(device));
    pytest.extend(["-o".to_owned(), format!("cache_dir={cache}")]);
    if options.verbose_tests {
        pytest.push("-s".to_owned());
    }
    pytest.extend(["--device".to_owned(), device.spec().name.to_owned()]);
    let mode_flags: &[&str] = match mode {
        TestMode::Headless => &[],
        TestMode::WithDisplay => &["--display"],
        TestMode::OnDevice => &["--display", "--backend", "ledgerwallet"],
        TestMode::GoldenRun => &["--golden_run"],
    };
    pytest.extend(mode_flags.iter().map(|flag| (*flag).to_owned()));
    steps.push(InContainer::new(pytest).venv());
    Ok(steps)
}

/// Checks each device on its own. When the target is every device of the app, the checks cover
/// all of them in one run.
fn enforcer(app: &App, devices: &[Device], target: &Target, options: &Options) -> Vec<Job> {
    let supported: BTreeSet<Device> = app.devices.iter().copied().collect();
    let whole_app = target.is_multiple() && *target.devices() == supported;

    let check = |device: Device, for_device: bool| {
        let mut argv = vec!["/opt/enforcer.sh".to_owned()];
        if let Some(check) = &options.enforcer_check {
            argv.extend(["-c".to_owned(), check.clone()]);
        }
        if for_device {
            argv.extend(["-t".to_owned(), device.sdk_model(app.language).to_owned()]);
        }
        InContainer::new(argv).sdk(device)
    };

    if whole_app {
        run_once(devices, vec![check(devices[0], false)])
    } else {
        devices
            .iter()
            .map(|&device| Job {
                device,
                steps: vec![check(device, true).into()],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::UseCaseOrigin::{BuiltIn, Manifest};
    use crate::rust_app::RustAppPackage;

    fn app(language: Language, devices: &[Device]) -> App {
        App {
            folder: "/work/app-x".into(),
            language,
            build_directory: "./".to_owned(),
            devices: devices.to_vec(),
            rust: (language == Language::Rust).then(|| RustAppPackage {
                package_name: "app-x-rust".to_owned(),
                app_name: "X".to_owned(),
            }),
            build_use_cases: Vec::new(),
            test_suites: Vec::new(),
            makefile: None,
            compose_file: None,
        }
    }

    fn c_app() -> App {
        app(Language::C, &[Device::Stax, Device::Flex, Device::NanoX])
    }

    fn rust_app() -> App {
        app(Language::Rust, &[Device::Stax, Device::NanoSPlus])
    }

    fn target(devices: &[Device]) -> Target {
        Target::new(devices.iter().copied()).unwrap()
    }

    fn use_case(name: &str, origin: UseCaseOrigin) -> Option<BuildUseCase> {
        Some(BuildUseCase {
            name: name.to_owned(),
            origin,
        })
    }

    fn plan(task: Task, app: &App, devices: &[Device], options: &Options) -> Plan {
        plan_task(task, app, &target(devices), options).unwrap()
    }

    /// The program and arguments of each step of each job.
    fn argvs(plan: &Plan) -> Vec<(Device, Vec<Vec<&str>>)> {
        plan.jobs
            .iter()
            .map(|job| {
                let steps = job
                    .steps
                    .iter()
                    .map(|step| container(step).argv.iter().map(String::as_str).collect())
                    .collect();
                (job.device, steps)
            })
            .collect()
    }

    fn only_step(plan: &Plan) -> &InContainer {
        assert_eq!(plan.jobs.len(), 1, "{plan:?}");
        assert_eq!(plan.jobs[0].steps.len(), 1, "{plan:?}");
        container(&plan.jobs[0].steps[0])
    }

    fn container(step: &Step) -> &InContainer {
        match step {
            Step::InContainer(step) => step,
            Step::OnHost(host) => {
                panic!("a step in a container, not on the host: {}", host.command)
            }
        }
    }

    fn tests_options(paths: &[&str]) -> Options {
        Options {
            tests: Some(TestRun {
                directory: "tests/standalone".to_owned(),
                paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            }),
            ..Options::default()
        }
    }

    #[test]
    fn a_c_app_is_built_with_make_in_each_device_container() {
        let plan = plan(
            Task::BuildApp,
            &c_app(),
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        assert_eq!(
            argvs(&plan),
            [
                (Device::Stax, vec![vec!["make", "-C", "./", "-j"]]),
                (Device::Flex, vec![vec!["make", "-C", "./", "-j"]]),
            ]
        );
        assert_eq!(container(&plan.jobs[0].steps[0]).sdk, Some(Device::Stax));
        assert_eq!(container(&plan.jobs[1].steps[0]).sdk, Some(Device::Flex));
    }

    #[test]
    fn the_build_use_case_picks_the_make_option() {
        let build = |use_case, variant| {
            let options = Options {
                build_use_case: use_case,
                variant,
                ..Options::default()
            };
            plan(Task::BuildApp, &c_app(), &[Device::Stax], &options)
        };
        let argv = |plan: &Plan| only_step(plan).argv[3..].to_vec();

        assert_eq!(argv(&build(None, None)), ["-j"]);
        assert_eq!(argv(&build(use_case("release", BuiltIn), None)), ["-j"]);
        assert_eq!(
            argv(&build(use_case("debug", BuiltIn), None)),
            ["-j", "DEBUG=1"]
        );
        assert_eq!(
            argv(&build(use_case("custom", Manifest), None)),
            ["-j", "custom"]
        );
        let variant = Some(Variant {
            name: "COIN".to_owned(),
            value: "ethereum".to_owned(),
        });
        assert_eq!(
            argv(&build(use_case("debug", BuiltIn), variant)),
            ["-j", "DEBUG=1", "COIN=ethereum"]
        );
    }

    #[test]
    fn a_debug_use_case_defined_by_the_manifest_is_built_as_that_make_target() {
        let options = Options {
            build_use_case: use_case("debug", Manifest),
            ..Options::default()
        };

        let plan = plan(Task::BuildApp, &c_app(), &[Device::Stax], &options);

        assert_eq!(only_step(&plan).argv, ["make", "-C", "./", "-j", "debug"]);
    }

    #[test]
    fn a_rust_app_is_built_with_cargo_ledger_and_its_artifacts_are_renamed() {
        let options = Options {
            build_use_case: use_case("debug", BuiltIn),
            ..Options::default()
        };

        let plan = plan(Task::BuildApp, &rust_app(), &[Device::NanoSPlus], &options);

        assert_eq!(
            argvs(&plan),
            [(
                Device::NanoSPlus,
                vec![
                    vec![
                        "cargo",
                        "ledger",
                        "build",
                        "nanosplus",
                        "--",
                        "--features",
                        "debug",
                        "-Zunstable-options",
                        "--artifact-dir=build/nanos2/bin"
                    ],
                    vec![
                        "mv",
                        "build/nanos2/bin/app-x-rust",
                        "build/nanos2/bin/app.elf"
                    ],
                    vec![
                        "mv",
                        "build/nanos2/bin/app-x-rust.apdu",
                        "build/nanos2/bin/app.apdu"
                    ],
                ]
            )]
        );
        assert!(
            plan.jobs[0]
                .steps
                .iter()
                .all(|s| container(s).dir.as_deref() == Some("./"))
        );
    }

    #[test]
    fn a_rust_app_without_its_cargo_names_cannot_be_built() {
        let mut app = rust_app();
        app.rust = None;

        let result = plan_task(
            Task::BuildApp,
            &app,
            &target(&[Device::Stax]),
            &Options::default(),
        );

        assert_eq!(result, Err(PlanError::MissingRustPackage));
    }

    #[test]
    fn clean_and_build_cleans_the_target_first_and_only_exists_for_c() {
        let plan = plan(
            Task::CleanAndBuild,
            &c_app(),
            &[Device::Stax],
            &Options::default(),
        );

        assert_eq!(
            argvs(&plan),
            [(
                Device::Stax,
                vec![
                    vec!["make", "-C", "./", "clean_target"],
                    vec!["make", "-C", "./", "-j"]
                ]
            )]
        );
        assert_eq!(
            plan_task(
                Task::CleanAndBuild,
                &rust_app(),
                &target(&[Device::Stax]),
                &Options::default()
            ),
            Err(PlanError::NotAvailable(Task::CleanAndBuild, Language::Rust))
        );
    }

    #[test]
    fn cleaning_every_build_runs_once_whatever_the_target() {
        let c = plan(
            Task::CleanAllBuilds,
            &c_app(),
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );
        let rust = plan(
            Task::CleanAllBuilds,
            &rust_app(),
            &[Device::Stax, Device::NanoSPlus],
            &Options::default(),
        );

        assert_eq!(
            argvs(&c),
            [(Device::Stax, vec![vec!["make", "-C", "./", "clean"]])]
        );
        assert_eq!(only_step(&c).sdk, None);
        assert_eq!(
            argvs(&rust),
            [(
                Device::NanoSPlus,
                vec![vec!["cargo", "clean"], vec!["rm", "-rf", "build"]]
            )]
        );
    }

    #[test]
    fn updating_the_rust_sdk_is_for_rust_apps_in_the_build_directory() {
        let plan = plan(
            Task::UpdateRustSdk,
            &rust_app(),
            &[Device::Stax],
            &Options::default(),
        );

        assert_eq!(
            only_step(&plan).argv,
            ["cargo", "update", "ledger_device_sdk"]
        );
        assert_eq!(only_step(&plan).dir.as_deref(), Some("./"));
        assert_eq!(
            plan_task(
                Task::UpdateRustSdk,
                &c_app(),
                &target(&[Device::Stax]),
                &Options::default()
            ),
            Err(PlanError::NotAvailable(Task::UpdateRustSdk, Language::C))
        );
    }

    #[test]
    fn the_emulator_runs_the_built_app_with_the_speculos_model() {
        let mut app = c_app();
        app.build_directory = "./app".to_owned();

        let plan = plan(
            Task::RunEmulator,
            &app,
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        assert_eq!(
            argvs(&plan),
            [
                (
                    Device::Stax,
                    vec![vec![
                        "speculos",
                        "--model",
                        "stax",
                        "./app/build/stax/bin/app.elf"
                    ]]
                ),
                (
                    Device::Flex,
                    vec![vec![
                        "speculos",
                        "--model",
                        "flex",
                        "./app/build/flex/bin/app.elf"
                    ]]
                ),
            ]
        );
    }

    #[test]
    fn the_emulator_of_nano_s_plus_uses_its_two_different_names() {
        let plan = plan(
            Task::RunEmulator,
            &c_app(),
            &[Device::NanoSPlus],
            &Options::default(),
        );

        assert_eq!(
            only_step(&plan).argv,
            ["speculos", "--model", "nanosp", "build/nanos2/bin/app.elf"]
        );
    }

    #[test]
    fn killing_the_emulator_is_done_in_every_selected_container() {
        let plan = plan(
            Task::KillEmulator,
            &c_app(),
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        assert_eq!(plan.jobs.len(), 2);
        assert!(
            plan.jobs
                .iter()
                .all(|job| container(&job.steps[0]).argv == ["pkill", "-f", "speculos"])
        );
    }

    #[test]
    fn tests_run_the_whole_suite_when_nothing_is_selected() {
        let plan = plan(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &[Device::Stax],
            &tests_options(&[]),
        );

        let steps: Vec<&InContainer> = plan.jobs[0].steps.iter().map(container).collect();
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].argv,
            [
                "bash",
                "-c",
                INSTALL_REQUIREMENTS_SCRIPT,
                "_",
                "tests/standalone/requirements.txt"
            ]
        );
        assert!(steps[0].continue_on_error && steps[0].venv);
        assert_eq!(
            steps[0].label.as_deref(),
            Some("pip install -r tests/standalone/requirements.txt"),
            "the log shows what it does, not the script"
        );
        assert_eq!(
            steps[1].argv,
            [
                "pytest",
                "tests/standalone",
                "--color=yes",
                "--tb=short",
                "-v",
                "-o",
                "cache_dir=/app/.pytest_cache/stax",
                "--device",
                "stax"
            ]
        );
        assert!(steps[1].venv && !steps[1].continue_on_error);
    }

    #[test]
    fn tests_run_only_the_selected_paths_on_each_device() {
        let paths = ["tests/standalone/test_a.py::test_x", "tests/standalone/sub"];

        let plan = plan(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &[Device::Stax, Device::Flex],
            &tests_options(&paths),
        );

        assert_eq!(plan.jobs.len(), 2);
        for job in &plan.jobs {
            let pytest = &container(job.steps.last().unwrap()).argv;
            assert_eq!(pytest[..3], ["pytest", paths[0], paths[1]]);
            assert_eq!(
                pytest[pytest.len() - 2..],
                ["--device", job.device.spec().name]
            );
        }
    }

    #[test]
    fn each_device_keeps_its_own_pytest_cache() {
        let plan = plan(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &[Device::NanoSPlus, Device::Flex],
            &tests_options(&[]),
        );

        let caches: Vec<String> = plan
            .jobs
            .iter()
            .map(|job| {
                let argv = &container(job.steps.last().unwrap()).argv;
                let at = argv.iter().position(|arg| arg == "-o").unwrap();
                argv[at + 1].clone()
            })
            .collect();
        assert_eq!(
            caches,
            [
                "cache_dir=/app/.pytest_cache/nanosp",
                "cache_dir=/app/.pytest_cache/flex"
            ]
        );
    }

    #[test]
    fn the_test_mode_chooses_the_extra_pytest_flags() {
        let flags = |mode, verbose| {
            let options = Options {
                verbose_tests: verbose,
                ..tests_options(&[])
            };
            let plan = plan(Task::RunTests(mode), &c_app(), &[Device::Stax], &options);
            container(plan.jobs[0].steps.last().unwrap()).argv[7..].to_vec()
        };

        assert_eq!(flags(TestMode::Headless, false), ["--device", "stax"]);
        assert_eq!(flags(TestMode::Headless, true), ["-s", "--device", "stax"]);
        assert_eq!(
            flags(TestMode::WithDisplay, false),
            ["--device", "stax", "--display"]
        );
        assert_eq!(
            flags(TestMode::GoldenRun, false),
            ["--device", "stax", "--golden_run"]
        );
        assert_eq!(
            flags(TestMode::OnDevice, false),
            ["--device", "stax", "--display", "--backend", "ledgerwallet"]
        );
    }

    #[test]
    fn extra_test_requirements_are_installed_first_and_do_not_block_the_tests() {
        let options = Options {
            test_requirements: Some("pip install extra".to_owned()),
            ..tests_options(&[])
        };

        let plan = plan(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &[Device::Stax],
            &options,
        );

        let first = container(&plan.jobs[0].steps[0]);
        assert_eq!(first.argv, ["bash", "-c", "pip install extra"]);
        assert!(first.continue_on_error && first.venv);
        assert_eq!(plan.jobs[0].steps.len(), 3);
    }

    #[test]
    fn tests_need_a_test_suite() {
        let result = plan_task(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &target(&[Device::Stax]),
            &Options::default(),
        );

        assert_eq!(result, Err(PlanError::NoTests));
    }

    #[test]
    fn tests_on_a_physical_device_need_exactly_one_device() {
        let task = Task::RunTests(TestMode::OnDevice);
        let several = plan_task(
            task,
            &c_app(),
            &target(&[Device::Stax, Device::Flex]),
            &tests_options(&[]),
        );
        let one = plan(task, &c_app(), &[Device::Flex], &tests_options(&[]));

        assert_eq!(several, Err(PlanError::NeedsOneDevice(task)));
        assert_eq!(one.jobs.len(), 1);
        assert_eq!(one.jobs[0].device, Device::Flex);
    }

    #[test]
    fn the_guideline_checks_run_per_device_with_the_sdk_name_of_the_language() {
        let options = Options {
            enforcer_check: Some("icons".to_owned()),
            ..Options::default()
        };

        let c = plan(Task::EnforcerChecks, &c_app(), &[Device::NanoX], &options);
        let rust = plan(
            Task::EnforcerChecks,
            &rust_app(),
            &[Device::NanoSPlus],
            &options,
        );

        assert_eq!(
            only_step(&c).argv,
            ["/opt/enforcer.sh", "-c", "icons", "-t", "nanox"]
        );
        assert_eq!(
            only_step(&rust).argv,
            ["/opt/enforcer.sh", "-c", "icons", "-t", "nanosplus"]
        );
        assert_eq!(only_step(&c).sdk, Some(Device::NanoX));
    }

    #[test]
    fn the_guideline_checks_cover_a_whole_app_in_one_run() {
        let app = c_app();

        let all = plan(
            Task::EnforcerChecks,
            &app,
            &[Device::Stax, Device::Flex, Device::NanoX],
            &Options::default(),
        );
        let some = plan(
            Task::EnforcerChecks,
            &app,
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        assert_eq!(only_step(&all).argv, ["/opt/enforcer.sh"]);
        assert_eq!(some.jobs.len(), 2);
    }

    #[test]
    fn a_terminal_is_opened_in_every_selected_container() {
        let plan = plan(
            Task::OpenTerminal,
            &c_app(),
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        assert_eq!(plan.jobs.len(), 2);
        for job in &plan.jobs {
            let shell = container(&job.steps[0]);
            assert_eq!(shell.argv, ["bash"]);
            assert!(shell.tty);
            assert_eq!(shell.sdk, Some(job.device));
        }
    }

    #[test]
    fn a_step_becomes_a_tagged_docker_exec_through_the_wrapper() {
        let step = InContainer::new(["make", "-C", "./", "-j"]).sdk(Device::Stax);
        let run_id = RunId::new("r1").unwrap();

        let command = step.command("app-x-container-stax", ContainerUser::Root, &run_id);

        assert_eq!(command.program, "docker");
        assert_eq!(
            command.args[..8],
            [
                "exec",
                "-i",
                "--user",
                "0",
                "-e",
                "LEDGER_RUN_ID=r1",
                "app-x-container-stax",
                "bash"
            ]
        );
        assert_eq!(command.args[8], "-c");
        assert_eq!(command.args[9], WRAPPER);
        assert_eq!(
            command.args[10..],
            ["_", "STAX_SDK", ".", "0", "make", "-C", "./", "-j"]
        );
    }

    #[test]
    fn a_terminal_step_allocates_a_tty_and_a_venv_step_says_so() {
        let step = InContainer::new(["bash"]).tty().venv().in_dir("tests");
        let command = step.command("c", ContainerUser::Root, &RunId::new("r").unwrap());

        assert_eq!(command.args[1], "-it");
        assert_eq!(command.args[10..], ["_", "", "tests", "1", "bash"]);
    }

    /// Runs the wrapper script itself, with a fake SDK variable.
    #[cfg(unix)]
    fn run_wrapper(sdk: &str, dir: &str, venv: &str, program: &[&str]) -> (bool, String) {
        let output = std::process::Command::new("bash")
            .args(["-c", WRAPPER, "_", sdk, dir, venv])
            .args(program)
            .env("NANOX_SDK", "/opt/nanox-secure-sdk")
            .env_remove("BOLOS_SDK")
            .env_remove("VIRTUAL_ENV")
            .output()
            .expect("bash runs");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_exports_the_sdk_named_by_the_variable() {
        let (ok, out) = run_wrapper(
            "NANOX_SDK",
            ".",
            "0",
            &["sh", "-c", "printf %s \"$BOLOS_SDK\""],
        );

        assert!(ok);
        assert_eq!(out, "/opt/nanox-secure-sdk");
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_sets_nothing_it_was_not_asked_to() {
        let (ok, out) = run_wrapper(
            "",
            ".",
            "0",
            &[
                "sh",
                "-c",
                "printf '[%s][%s]' \"$BOLOS_SDK\" \"$VIRTUAL_ENV\"",
            ],
        );

        assert!(ok);
        assert_eq!(out, "[][]");
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_puts_the_python_environment_first_when_asked() {
        let (ok, out) = run_wrapper(
            "",
            ".",
            "1",
            &["sh", "-c", "printf '%s %s' \"$VIRTUAL_ENV\" \"$PATH\""],
        );

        assert!(ok);
        assert!(out.starts_with("/opt/venv /opt/venv/bin:"), "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_runs_in_the_directory_and_fails_when_it_is_missing() {
        let (ok, out) = run_wrapper("", "/tmp", "0", &["pwd", "-P"]);
        assert!(ok);
        assert!(out.trim_end().ends_with("tmp"), "{out}");

        let (ok, _) = run_wrapper("", "/definitely/not/here", "0", &["true"]);
        assert!(!ok);
    }

    #[cfg(unix)]
    #[test]
    fn the_wrapper_keeps_every_argument_intact() {
        let (ok, out) = run_wrapper("", ".", "0", &["printf", "%s|", "a b", "$HOME", "it's", ""]);

        assert!(ok);
        assert_eq!(out, "a b|$HOME|it's||");
    }

    #[test]
    fn the_tests_run_from_the_app_folder_as_the_swap_suites_expect() {
        let plan = plan(
            Task::RunTests(TestMode::Headless),
            &c_app(),
            &[Device::Stax],
            &tests_options(&[]),
        );

        assert!(
            plan.jobs[0]
                .steps
                .iter()
                .all(|step| container(step).dir.is_none())
        );
    }

    fn compose_app() -> App {
        App {
            compose_file: Some("docker/docker-compose.yml".to_owned()),
            ..c_app()
        }
    }

    fn compose_options() -> Options {
        Options {
            compose: Some(ComposeRun {
                service: "builder".to_owned(),
                user: ContainerUser::Host {
                    uid: 1000,
                    gid: 1000,
                },
            }),
            ..Options::default()
        }
    }

    #[test]
    fn the_compose_terminal_is_typed_into_and_runs_on_the_host_once() {
        let plan = plan(
            Task::ComposeTerminal,
            &compose_app(),
            &[Device::Stax, Device::Flex],
            &compose_options(),
        );

        assert_eq!(plan.jobs.len(), 1);
        let Step::OnHost(host) = &plan.jobs[0].steps[0] else {
            panic!("{plan:?}")
        };
        assert!(host.tty);
        assert_eq!(
            host.command.shell_line(),
            "docker compose -f /work/app-x/docker/docker-compose.yml run --rm --remove-orphans --user 1000:1000 builder"
        );
    }

    #[test]
    fn the_compose_terminal_needs_a_compose_file_and_a_service() {
        let target = target(&[Device::Stax]);

        let no_file = plan_task(Task::ComposeTerminal, &c_app(), &target, &compose_options());
        let no_service = plan_task(
            Task::ComposeTerminal,
            &compose_app(),
            &target,
            &Options::default(),
        );

        assert_eq!(no_file.unwrap_err(), PlanError::NoComposeFile);
        assert_eq!(no_service.unwrap_err(), PlanError::NoComposeService);
    }

    #[cfg(unix)]
    mod collect_script {
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::process::Command as Process;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use super::*;

        /// A `pytest` that prints what a real one prints for `--help` and `--collect-only`.
        fn fake_pytest(custom_options: &str, status: i32) -> PathBuf {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ledger-collect-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(dir.join("bin")).unwrap();
            std::fs::create_dir_all(dir.join("suite")).unwrap();
            let script = format!(
                "#!/bin/sh\n\
                 case \"$1\" in\n\
                 --help) printf 'usage: pytest\\n\\nCustom options:\\n{custom_options}\\n\\nother:\\n  --foo\\n' ;;\n\
                 --collect-only) echo \"collected with: $*\"; echo 'test_a.py::test_x[stax]'; exit {status} ;;\n\
                 esac\n"
            );
            let path = dir.join("bin").join("pytest");
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            dir
        }

        fn collect(dir: &Path) -> (i32, String) {
            let path = format!(
                "{}:{}",
                dir.join("bin").display(),
                std::env::var("PATH").unwrap()
            );
            let output = Process::new("bash")
                .args(["-c", COLLECT_SCRIPT, "_"])
                .arg(dir.join("suite"))
                .arg("stax")
                .env("PATH", path)
                .output()
                .unwrap();
            (
                output.status.code().unwrap(),
                String::from_utf8(output.stdout).unwrap(),
            )
        }

        #[test]
        fn the_device_option_of_the_suite_gets_the_device() {
            let dir = fake_pytest("  --device=DEVICE   the device", 0);
            assert_eq!(
                collect(&dir),
                (
                    0,
                    "collected with: --collect-only -q --device stax\ntest_a.py::test_x[stax]\n"
                        .to_owned()
                )
            );
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn an_older_suite_that_calls_it_model_gets_the_device_as_its_model() {
            let dir = fake_pytest("  --model {nanos,nanox}   the model", 0);
            let (status, output) = collect(&dir);
            assert_eq!(status, 0);
            assert!(
                output.contains("--collect-only -q --model stax"),
                "{output}"
            );
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn a_suite_without_a_device_option_is_collected_as_it_is() {
            let dir = fake_pytest("  --golden_run   regenerate", 0);
            let (_, output) = collect(&dir);
            assert!(
                output.contains("collected with: --collect-only -q\n"),
                "{output}"
            );
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn no_test_found_is_not_a_failure_but_a_real_failure_is() {
            let none = fake_pytest("  --device=DEVICE   d1", 5);
            let broken = fake_pytest("  --device=DEVICE   device two", 2);

            assert_eq!(collect(&none).0, 0);
            assert_eq!(collect(&broken).0, 2);
            std::fs::remove_dir_all(none).unwrap();
            std::fs::remove_dir_all(broken).unwrap();
        }

        #[test]
        fn the_step_runs_the_script_in_the_python_environment_of_the_image() {
            let step = collect_tests("./tests/swap", Device::Flex);

            assert!(step.venv);
            assert_eq!(step.argv[..2], ["bash", "-c"]);
            assert_eq!(step.argv[3..], ["_", "./tests/swap", "flex"]);
        }
    }

    #[cfg(unix)]
    mod install_script {
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::process::{Child, Command as Process, Stdio};
        use std::sync::atomic::{AtomicUsize, Ordering};

        use super::*;

        /// A directory with a `pip` that records when it starts and ends, and a pip cache.
        struct Sandbox {
            dir: PathBuf,
        }

        impl Sandbox {
            fn new() -> Self {
                static NEXT: AtomicUsize = AtomicUsize::new(0);
                let dir = std::env::temp_dir().join(format!(
                    "ledger-install-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(dir.join("bin")).unwrap();
                std::fs::create_dir_all(dir.join("cache")).unwrap();
                let pip = dir.join("bin").join("pip");
                std::fs::write(
                    &pip,
                    "#!/bin/sh\necho \"start $$\" >> \"$FAKE_PIP_LOG\"\nsleep 0.4\necho \"end $$\" >> \"$FAKE_PIP_LOG\"\necho \"pip $*\"\n",
                )
                .unwrap();
                std::fs::set_permissions(&pip, std::fs::Permissions::from_mode(0o755)).unwrap();
                std::fs::write(dir.join("requirements.txt"), "pytest\n").unwrap();
                Self { dir }
            }

            fn start(&self, requirements: &Path, cache: &Path) -> Child {
                let path = format!(
                    "{}:{}",
                    self.dir.join("bin").display(),
                    std::env::var("PATH").unwrap()
                );
                Process::new("bash")
                    .args(["-c", INSTALL_REQUIREMENTS_SCRIPT, "_"])
                    .arg(requirements)
                    .env("PATH", path)
                    .env("PIP_CACHE_DIR", cache)
                    .env("FAKE_PIP_LOG", self.dir.join("pip.log"))
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap()
            }

            fn log(&self) -> Vec<String> {
                std::fs::read_to_string(self.dir.join("pip.log"))
                    .unwrap_or_default()
                    .lines()
                    .map(|line| line.split(' ').next().unwrap().to_owned())
                    .collect()
            }

            fn finish(self) {
                std::fs::remove_dir_all(self.dir).unwrap();
            }
        }

        fn output(child: Child) -> String {
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        }

        #[test]
        fn the_requirements_file_is_installed() {
            let sandbox = Sandbox::new();

            let printed = output(sandbox.start(
                &sandbox.dir.join("requirements.txt"),
                &sandbox.dir.join("cache"),
            ));

            assert!(printed.contains("pip install -r"), "{printed}");
            assert_eq!(sandbox.log(), ["start", "end"]);
            sandbox.finish();
        }

        #[test]
        fn a_suite_without_requirements_installs_nothing_and_succeeds() {
            let sandbox = Sandbox::new();

            let printed =
                output(sandbox.start(&sandbox.dir.join("none.txt"), &sandbox.dir.join("cache")));

            assert_eq!(printed, "");
            assert!(sandbox.log().is_empty());
            sandbox.finish();
        }

        #[test]
        fn containers_install_one_after_the_other_and_the_one_that_waits_says_so() {
            let sandbox = Sandbox::new();
            let requirements = sandbox.dir.join("requirements.txt");
            let cache = sandbox.dir.join("cache");

            let first = sandbox.start(&requirements, &cache);
            let second = sandbox.start(&requirements, &cache);
            let printed = [output(first), output(second)];

            let log = sandbox.log();
            assert_eq!(log.len(), 4, "{log:?}");
            assert_eq!(
                log,
                ["start", "end", "start", "end"],
                "never at the same time"
            );
            let waiting = printed
                .iter()
                .filter(|text| text.contains("Waiting for another container"))
                .count();
            assert_eq!(waiting, 1, "{printed:?}");
            sandbox.finish();
        }

        #[test]
        fn without_a_pip_cache_to_write_to_it_installs_without_a_lock() {
            let sandbox = Sandbox::new();

            let printed = output(sandbox.start(
                &sandbox.dir.join("requirements.txt"),
                &sandbox.dir.join("no-such-cache"),
            ));

            assert!(printed.contains("pip install -r"), "{printed}");
            assert_eq!(sandbox.log(), ["start", "end"]);
            sandbox.finish();
        }
    }
}
