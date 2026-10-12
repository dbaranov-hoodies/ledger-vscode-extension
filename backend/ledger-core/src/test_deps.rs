//! Preparing the apps that a test suite needs. A suite that ships helper scripts for it gets them
//! run: they know what to clone and build, so the dependencies of the manifest are only a hint.
//! A suite that does not, but whose `conftest.py` loads apps that the extension knows where to
//! get (the Exchange tests), gets them built by the extension, see [`crate::sideloaded`].

use crate::app::{App, join_path};
use crate::command::Command;
use crate::device::Device;
use crate::host_python::{HostPython, path_text};
use crate::sideloaded::{self, Sideloaded};
use crate::task::{InContainer, PlanError, Step};

pub const CLONE_SCRIPT: &str = "helper_tool_clone_dependencies.py";
pub const BUILD_SCRIPT: &str = "helper_tool_build_dependencies.py";
/// Where the scripts put what they clone and build, inside the suite directory.
const DEPENDENCIES_DIR: &str = ".test_dependencies";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestDependencies {
    /// The suite directory, relative to the app folder. Both scripts are in it, when it has them.
    pub suite_directory: String,
    /// The suite ships the scripts, which then do the work.
    pub scripts: bool,
    /// The apps the suite loads, for a suite without scripts.
    pub sideloaded: Option<Sideloaded>,
}

impl TestDependencies {
    /// Whether the extension can prepare the suite, with its scripts or by itself.
    pub fn can_be_prepared(&self) -> bool {
        self.scripts
            || self
                .sideloaded
                .as_ref()
                .is_some_and(Sideloaded::is_buildable)
    }
}

/// The scripts a suite needs to be prepared, relative to the app folder: the caller checks that
/// both exist.
pub fn scripts(suite_directory: &str) -> [String; 2] {
    [CLONE_SCRIPT, BUILD_SCRIPT].map(|script| join_path(suite_directory, script))
}

/// The steps that prepare the suite for these devices. The scripts of the suite come first, and
/// need the Python environment of the host.
pub(crate) fn steps(
    dependencies: &TestDependencies,
    clean: bool,
    app: &App,
    devices: &[Device],
    host: Option<&HostPython>,
) -> Result<Vec<Step>, PlanError> {
    if dependencies.scripts {
        let host = host.ok_or(PlanError::NoHostPython)?;
        return Ok(script_steps(dependencies, clean, app, host));
    }
    match &dependencies.sideloaded {
        Some(sideloaded) if sideloaded.is_buildable() => Ok(sideloaded::steps(
            app,
            &dependencies.suite_directory,
            sideloaded,
            devices,
            clean,
        )),
        _ => Err(PlanError::NoTestDependencies),
    }
}

/// Clones on the host, where the Git credentials are (GitPython goes into its Python environment),
/// then builds in the container, which has the SDKs.
fn script_steps(
    dependencies: &TestDependencies,
    clean: bool,
    app: &App,
    host: &HostPython,
) -> Vec<Step> {
    let [clone, build] = scripts(&dependencies.suite_directory);
    let mut steps = Vec::new();
    if clean {
        let prepared = join_path(&dependencies.suite_directory, DEPENDENCIES_DIR);
        steps.push(InContainer::new(["rm", "-rf", &prepared]).into());
    }
    steps.extend(host.environment(&["GitPython"]));
    let clone_on_host = app.folder.join(clone.trim_start_matches("./"));
    steps.push(Step::on_host(
        Command::new(host.python()).arg(path_text(&clone_on_host)),
    ));
    steps.push(InContainer::new(["python3", &build]).into());
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Device;
    use crate::language::Language;
    use crate::platform::Platform;
    use crate::target::Target;
    use crate::task::{Options, PlanError, Task, plan_task};

    fn app() -> App {
        App {
            folder: "/work/app-x".into(),
            language: Language::C,
            build_directory: "./".to_owned(),
            devices: vec![Device::Stax, Device::Flex],
            rust: None,
            build_use_cases: Vec::new(),
            test_suites: Vec::new(),
            makefile: None,
            compose_file: None,
        }
    }

    fn options() -> Options {
        Options {
            host: Some(HostPython {
                platform: Platform::Linux,
                venv: "/home/u/venvs/app-x".into(),
            }),
            test_dependencies: Some(TestDependencies {
                suite_directory: "./tests/swap/".to_owned(),
                scripts: true,
                sideloaded: None,
            }),
            ..Options::default()
        }
    }

    fn loaded(variants: &[&str]) -> Sideloaded {
        Sideloaded {
            directory: "tests/libs".to_owned(),
            apps: variants
                .iter()
                .map(|variant| sideloaded::SideloadedApp {
                    variant: (*variant).to_owned(),
                    name: (*variant).to_owned(),
                })
                .collect(),
        }
    }

    /// A suite without scripts, which loads these apps. There is no Python environment on the host.
    fn options_without_scripts(variants: &[&str]) -> Options {
        Options {
            host: None,
            test_dependencies: Some(TestDependencies {
                suite_directory: "./tests/".to_owned(),
                scripts: false,
                sideloaded: Some(loaded(variants)),
            }),
            ..Options::default()
        }
    }

    fn plan(clean: bool, options: &Options) -> Result<crate::task::Plan, PlanError> {
        let target = Target::new([Device::Stax, Device::Flex]).unwrap();
        let task = Task::PrepareTestDependencies { clean };
        plan_task(task, &app(), &target, options)
    }

    #[test]
    fn the_scripts_are_in_the_suite_directory() {
        assert_eq!(
            scripts("./tests/swap/"),
            [
                "./tests/swap/helper_tool_clone_dependencies.py",
                "./tests/swap/helper_tool_build_dependencies.py"
            ]
        );
        assert_eq!(scripts("./")[0], "helper_tool_clone_dependencies.py");
    }

    #[test]
    fn the_apps_are_cloned_on_the_host_and_then_built_in_a_container() {
        let plan = plan(false, &options()).unwrap();

        assert_eq!(plan.jobs.len(), 1, "once, not once per device");
        let steps = &plan.jobs[0].steps;
        assert_eq!(steps.len(), 4);
        let Step::OnHost(venv) = &steps[0] else {
            panic!("{steps:?}")
        };
        assert_eq!(
            venv.command.shell_line(),
            "python3 -m venv /home/u/venvs/app-x"
        );
        let Step::OnHost(git) = &steps[1] else {
            panic!("{steps:?}")
        };
        assert!(
            git.command
                .shell_line()
                .ends_with("pip install --disable-pip-version-check GitPython")
        );
        let Step::OnHost(clone) = &steps[2] else {
            panic!("{steps:?}")
        };
        assert_eq!(
            clone.command.shell_line(),
            "/home/u/venvs/app-x/bin/python /work/app-x/tests/swap/helper_tool_clone_dependencies.py"
        );
        assert!(!clone.tty);
        let Step::InContainer(build) = &steps[3] else {
            panic!("{steps:?}")
        };
        assert_eq!(
            build.argv,
            ["python3", "./tests/swap/helper_tool_build_dependencies.py"]
        );
        assert_eq!(
            build.dir, None,
            "from the app folder, as the swap tests run"
        );
    }

    #[test]
    fn a_clean_start_removes_what_was_prepared_first() {
        let plan = plan(true, &options()).unwrap();

        let steps = &plan.jobs[0].steps;
        assert_eq!(steps.len(), 5);
        let Step::InContainer(remove) = &steps[0] else {
            panic!("{steps:?}")
        };
        assert_eq!(
            remove.argv,
            ["rm", "-rf", "./tests/swap/.test_dependencies"]
        );
    }

    #[test]
    fn the_suite_and_the_host_environment_are_needed() {
        let no_suite = Options {
            test_dependencies: None,
            ..options()
        };
        let no_host = Options {
            host: None,
            ..options()
        };

        assert_eq!(
            plan(false, &no_suite).unwrap_err(),
            PlanError::NoTestDependencies
        );
        assert_eq!(plan(false, &no_host).unwrap_err(), PlanError::NoHostPython);
    }

    #[test]
    fn a_suite_without_scripts_is_prepared_by_the_extension_for_the_selected_devices() {
        let plan = plan(false, &options_without_scripts(&["tezos"])).unwrap();

        assert_eq!(plan.jobs.len(), 1, "once, not once per device");
        let steps = &plan.jobs[0].steps;
        // The ignore file, the four Git commands, a build for each of the two devices, the check.
        assert_eq!(steps.len(), 1 + 4 + 2 + 1, "{steps:?}");
        let builds: Vec<_> = steps
            .iter()
            .filter_map(|step| match step {
                Step::InContainer(container) => container.sdk,
                Step::OnHost(_) => None,
            })
            .collect();
        assert_eq!(builds, [Device::Stax, Device::Flex]);
    }

    #[test]
    fn the_scripts_of_a_suite_win_over_what_its_conftest_loads() {
        let mut both = options();
        both.test_dependencies.as_mut().unwrap().sideloaded = Some(loaded(&["tezos"]));

        let plan = plan(false, &both).unwrap();

        assert_eq!(
            plan.jobs[0].steps.len(),
            4,
            "the venv, GitPython, clone, build"
        );
    }

    #[test]
    fn a_suite_that_loads_nothing_the_extension_can_build_cannot_be_prepared() {
        for options in [
            options_without_scripts(&[]),
            options_without_scripts(&["unheard_of"]),
        ] {
            assert_eq!(
                plan(false, &options).unwrap_err(),
                PlanError::NoTestDependencies
            );
            assert!(!options.test_dependencies.unwrap().can_be_prepared());
        }
        assert!(
            options_without_scripts(&["tezos"])
                .test_dependencies
                .unwrap()
                .can_be_prepared()
        );
        assert!(options().test_dependencies.unwrap().can_be_prepared());
    }
}
