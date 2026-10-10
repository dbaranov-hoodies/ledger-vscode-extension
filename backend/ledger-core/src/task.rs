//! What each action of the extension runs, as commands for the containers of a target.
//!
//! A [`Plan`] has one [`Job`] per device. The jobs are independent and run side by side, each in
//! the container of its device, and the steps of a job run one after the other.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::app::{App, join_path};
use crate::command::Command;
use crate::device::Device;
use crate::docker::{self, ContainerUser, RunId};
use crate::language::Language;
use crate::manifest::{BuildUseCase, UseCaseOrigin};
use crate::target::Target;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub device: Device,
    /// Run in the container of the device, in order. The job stops at the first step that fails,
    /// unless that step keeps going.
    pub steps: Vec<InContainer>,
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
        docker::exec_tagged(container, user, run_id, self.tty, &script)
            .label(shell_words::join(&self.argv))
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
        Task::CleanAllBuilds => one_container(&devices, clean_all(app)),
        Task::UpdateRustSdk => {
            require(task, app, Language::Rust)?;
            let update = InContainer::new(["cargo", "update", "ledger_device_sdk"])
                .in_dir(&app.build_directory);
            one_container(&devices, vec![update])
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
    };
    Ok(Plan { jobs })
}

fn require(task: Task, app: &App, language: Language) -> Result<(), PlanError> {
    if app.language == language {
        Ok(())
    } else {
        Err(PlanError::NotAvailable(task, app.language))
    }
}

fn per_device(
    devices: &[Device],
    steps: impl Fn(Device) -> Result<Vec<InContainer>, PlanError>,
) -> Result<Vec<Job>, PlanError> {
    devices
        .iter()
        .map(|&device| {
            Ok(Job {
                device,
                steps: steps(device)?,
            })
        })
        .collect()
}

/// For actions that are not about a device: run once, in the container of the first device.
fn one_container(devices: &[Device], steps: Vec<InContainer>) -> Vec<Job> {
    vec![Job {
        device: devices[0],
        steps,
    }]
}

fn single_device(
    task: Task,
    target: &Target,
    steps: impl FnOnce(Device) -> Result<Vec<InContainer>, PlanError>,
) -> Result<Vec<Job>, PlanError> {
    let device = target.single().ok_or(PlanError::NeedsOneDevice(task))?;
    Ok(vec![Job {
        device,
        steps: steps(device)?,
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
            r#"[ ! -f "$1" ] || pip install -r "$1""#,
            "_",
            &requirements_file,
        ])
        .venv()
        .keep_going(),
    );

    let mut pytest = vec!["pytest".to_owned()];
    if run.paths.is_empty() {
        pytest.push(run.directory.clone());
    } else {
        pytest.extend(run.paths.iter().cloned());
    }
    pytest.extend(["--color=yes", "--tb=short", "-v"].map(str::to_owned));
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
        one_container(devices, vec![check(devices[0], false)])
    } else {
        devices
            .iter()
            .map(|&device| Job {
                device,
                steps: vec![check(device, true)],
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
            folder_name: "app-x".to_owned(),
            language,
            build_directory: "./".to_owned(),
            devices: devices.to_vec(),
            rust: (language == Language::Rust).then(|| RustAppPackage {
                package_name: "app-x-rust".to_owned(),
                app_name: "X".to_owned(),
            }),
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
                    .map(|step| step.argv.iter().map(String::as_str).collect())
                    .collect();
                (job.device, steps)
            })
            .collect()
    }

    fn only_step(plan: &Plan) -> &InContainer {
        assert_eq!(plan.jobs.len(), 1, "{plan:?}");
        assert_eq!(plan.jobs[0].steps.len(), 1, "{plan:?}");
        &plan.jobs[0].steps[0]
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
        assert_eq!(plan.jobs[0].steps[0].sdk, Some(Device::Stax));
        assert_eq!(plan.jobs[1].steps[0].sdk, Some(Device::Flex));
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
                .all(|s| s.dir.as_deref() == Some("./"))
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
                .all(|job| job.steps[0].argv == ["pkill", "-f", "speculos"])
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

        let steps = &plan.jobs[0].steps;
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].argv,
            [
                "bash",
                "-c",
                r#"[ ! -f "$1" ] || pip install -r "$1""#,
                "_",
                "tests/standalone/requirements.txt"
            ]
        );
        assert!(steps[0].continue_on_error && steps[0].venv);
        assert_eq!(
            steps[1].argv,
            [
                "pytest",
                "tests/standalone",
                "--color=yes",
                "--tb=short",
                "-v",
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
            let pytest = &job.steps.last().unwrap().argv;
            assert_eq!(pytest[..3], ["pytest", paths[0], paths[1]]);
            assert_eq!(
                pytest[pytest.len() - 2..],
                ["--device", job.device.spec().name]
            );
        }
    }

    #[test]
    fn the_test_mode_chooses_the_extra_pytest_flags() {
        let flags = |mode, verbose| {
            let options = Options {
                verbose_tests: verbose,
                ..tests_options(&[])
            };
            let plan = plan(Task::RunTests(mode), &c_app(), &[Device::Stax], &options);
            plan.jobs[0].steps.last().unwrap().argv[5..].to_vec()
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

        let first = &plan.jobs[0].steps[0];
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
            assert_eq!(job.steps[0].argv, ["bash"]);
            assert!(job.steps[0].tty);
            assert_eq!(job.steps[0].sdk, Some(job.device));
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
}
