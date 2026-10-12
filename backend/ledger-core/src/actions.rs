//! The actions the user can start, as one list: what each is, where the UI shows it, whether it
//! can run now and what it runs. The ids are the names the old extension gave its tasks.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::app::App;
use crate::device_ops::{self, DeviceTask};
use crate::docker::ContainerUser;
use crate::environment::Environment;
use crate::language::Language;
use crate::manifest::TestSuite;
use crate::selection::AppSelection;
use crate::task::{self, ComposeRun, Options, Task, TestMode, TestRun, Variant};
use crate::test_deps::TestDependencies;
use crate::test_tree::TestSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ActionId {
    #[serde(rename = "Update Container")]
    UpdateContainer,
    #[serde(rename = "Create Container")]
    CreateContainer,
    #[serde(rename = "Open Terminal")]
    OpenTerminal,
    #[serde(rename = "Open Compose Terminal")]
    OpenComposeTerminal,
    #[serde(rename = "Enforcer Checks")]
    EnforcerChecks,
    #[serde(rename = "AI Review")]
    AiReview,
    #[serde(rename = "Build App")]
    BuildApp,
    #[serde(rename = "Clean & Build")]
    CleanAndBuild,
    #[serde(rename = "Update Rust SDK")]
    UpdateRustSdk,
    #[serde(rename = "Clean Target Build")]
    CleanTargetBuild,
    #[serde(rename = "Clean All Builds")]
    CleanAllBuilds,
    #[serde(rename = "Run with Emulator")]
    RunEmulator,
    #[serde(rename = "Kill Emulator")]
    KillEmulator,
    #[serde(rename = "Run Tests")]
    RunTests,
    #[serde(rename = "Tests with Display")]
    TestsWithDisplay,
    #[serde(rename = "Tests on Device")]
    TestsOnDevice,
    #[serde(rename = "Generate Snapshots")]
    GenerateSnapshots,
    #[serde(rename = "Prepare Test Dependencies")]
    PrepareTestDependencies,
    #[serde(rename = "Rebuild Test Dependencies")]
    RebuildTestDependencies,
    #[serde(rename = "Load on Device")]
    LoadOnDevice,
    #[serde(rename = "Delete App from Device")]
    DeleteAppFromDevice,
    #[serde(rename = "Quick Device Setup")]
    QuickDeviceSetup,
}

/// The sections of the UI, in the order it shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Group {
    Build,
    Emulator,
    Tests,
    Device,
    Tools,
}

impl Group {
    pub const ALL: [Self; 5] = [
        Self::Build,
        Self::Emulator,
        Self::Tests,
        Self::Device,
        Self::Tools,
    ];

    fn icon(self) -> &'static str {
        match self {
            Self::Build => "tools",
            Self::Emulator => "play",
            Self::Tests => "beaker",
            Self::Device => "device-mobile",
            Self::Tools => "terminal",
        }
    }
}

/// What starting an action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// Planned by [`task::plan_task`] and run for the selected devices.
    Task(Task),
    /// Changes the containers themselves.
    Containers(ContainerAction),
    /// Needs VS Code, so the shell runs it.
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    /// Pulls the image, then creates the containers again.
    Update,
    /// Creates the containers again from the image that is already there.
    Create,
}

struct Spec {
    id: ActionId,
    group: Group,
    label: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    /// The action of its group that has a button of its own.
    main: bool,
    /// The language of the apps it is for, if not both.
    language: Option<Language>,
    /// Not for several devices at once.
    one_device: bool,
    run: Run,
}

const fn spec(
    id: ActionId,
    group: Group,
    label: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    run: Run,
) -> Spec {
    Spec {
        id,
        group,
        label,
        icon,
        tooltip,
        main: false,
        language: None,
        one_device: false,
        run,
    }
}

impl Spec {
    const fn main(mut self) -> Self {
        self.main = true;
        self
    }

    const fn only(mut self, language: Language) -> Self {
        self.language = Some(language);
        self
    }

    const fn one_device(mut self) -> Self {
        self.one_device = true;
        self
    }
}

/// In the order the UI lists them within a group.
const SPECS: [Spec; 21] = [
    spec(
        ActionId::BuildApp,
        Group::Build,
        "Build App",
        "tools",
        "Build app incrementally (faster, only rebuilds changed files)",
        Run::Task(Task::BuildApp),
    )
    .main(),
    spec(
        ActionId::CleanAndBuild,
        Group::Build,
        "Clean & Build",
        "sync",
        "Clean and build application from scratch (guarantees fresh build)",
        Run::Task(Task::CleanAndBuild),
    )
    .only(Language::C),
    spec(
        ActionId::UpdateRustSdk,
        Group::Build,
        "Update Rust SDK",
        "cloud-download",
        "Update ledger_device_sdk to the latest version in Cargo.lock",
        Run::Task(Task::UpdateRustSdk),
    )
    .only(Language::Rust),
    spec(
        ActionId::CleanTargetBuild,
        Group::Build,
        "Clean Target Build",
        "trash",
        "Clean the app build files for the selected target",
        Run::Task(Task::CleanTargetBuild),
    )
    .only(Language::C)
    .one_device(),
    spec(
        ActionId::CleanAllBuilds,
        Group::Build,
        "Clean All Builds",
        "clear-all",
        "Clean the app build files for all targets",
        Run::Task(Task::CleanAllBuilds),
    ),
    spec(
        ActionId::RunEmulator,
        Group::Emulator,
        "Run with Emulator",
        "play",
        "Run app with emulator (Speculos)",
        Run::Task(Task::RunEmulator),
    )
    .main()
    .one_device(),
    spec(
        ActionId::KillEmulator,
        Group::Emulator,
        "Kill Emulator",
        "debug-stop",
        "Kill emulator (Speculos) instance",
        Run::Task(Task::KillEmulator),
    )
    .one_device(),
    spec(
        ActionId::RunTests,
        Group::Tests,
        "Run Tests",
        "beaker",
        "Run Python functional tests (with Qt display disabled)",
        Run::Task(Task::RunTests(TestMode::Headless)),
    )
    .main(),
    spec(
        ActionId::TestsWithDisplay,
        Group::Tests,
        "Tests with Display",
        "eye",
        "Run Python functional tests (with Qt display enabled)",
        Run::Task(Task::RunTests(TestMode::WithDisplay)),
    ),
    spec(
        ActionId::TestsOnDevice,
        Group::Tests,
        "Tests on Device",
        "device-mobile",
        "Run Python functional tests (with Qt display enabled) on real device",
        Run::Task(Task::RunTests(TestMode::OnDevice)),
    )
    .one_device(),
    spec(
        ActionId::GenerateSnapshots,
        Group::Tests,
        "Generate Snapshots",
        "file-media",
        "Run Python functional tests with '--golden_run' option to generate golden snapshots. They are used during tests runs to check what should be displayed on the device screen",
        Run::Task(Task::RunTests(TestMode::GoldenRun)),
    ),
    spec(
        ActionId::PrepareTestDependencies,
        Group::Tests,
        "Prepare Test Dependencies",
        "package",
        "Clone and build the apps that the tests of the selected suite need: with the scripts of the suite (for example Exchange and Ethereum for swap tests) or, when it has none, the apps its conftest.py loads (the coin apps of the Exchange tests)",
        Run::Task(Task::PrepareTestDependencies { clean: false }),
    ),
    spec(
        ActionId::RebuildTestDependencies,
        Group::Tests,
        "Rebuild Test Dependencies",
        "refresh",
        "Remove the prepared dependencies of the selected suite, then clone and build them again",
        Run::Task(Task::PrepareTestDependencies { clean: true }),
    ),
    spec(
        ActionId::LoadOnDevice,
        Group::Device,
        "Load on Device",
        "cloud-upload",
        "Load app on a physical device",
        Run::Task(Task::Device(DeviceTask::Load)),
    )
    .main()
    .one_device(),
    spec(
        ActionId::DeleteAppFromDevice,
        Group::Device,
        "Delete App from Device",
        "trash",
        "Delete app from a physical device",
        Run::Task(Task::Device(DeviceTask::Delete)),
    )
    .one_device(),
    spec(
        ActionId::QuickDeviceSetup,
        Group::Device,
        "Quick Device Setup",
        "zap",
        "Automatic initial device setup with pre-defined test seed and PIN",
        Run::Task(Task::Device(DeviceTask::QuickSetup)),
    )
    .one_device(),
    spec(
        ActionId::OpenTerminal,
        Group::Tools,
        "Open Terminal",
        "terminal",
        "Open terminal in container with default configuration",
        Run::Task(Task::OpenTerminal),
    )
    .main(),
    spec(
        ActionId::UpdateContainer,
        Group::Tools,
        "Update Container",
        "desktop-download",
        "Update docker container (pull image and restart container)",
        Run::Containers(ContainerAction::Update),
    ),
    spec(
        ActionId::CreateContainer,
        Group::Tools,
        "Create Container",
        "vm",
        "Create docker container from existing local image (without pulling)",
        Run::Containers(ContainerAction::Create),
    ),
    spec(
        ActionId::OpenComposeTerminal,
        Group::Tools,
        "Open Compose Terminal",
        "terminal-bash",
        "Open terminal in container with 'docker-compose.yml' configuration",
        Run::Task(Task::ComposeTerminal),
    )
    .one_device(),
    spec(
        ActionId::EnforcerChecks,
        Group::Tools,
        "Enforcer Checks",
        "checklist",
        "Run Guideline Enforcer checks. These checks are also run in the app's repository CI and must pass before the app can be deployed.",
        Run::Task(Task::EnforcerChecks),
    ),
];

/// Needs the AI review of the shell, so it is not in [`SPECS`] with a task of the core.
const AI_REVIEW: Spec = spec(
    ActionId::AiReview,
    Group::Tools,
    "AI Review",
    "sparkle",
    "Run AI-powered code review on current branch changes using Ledger's code analysis rules.",
    Run::Shell,
);

fn specs() -> impl Iterator<Item = &'static Spec> {
    SPECS.iter().chain(std::iter::once(&AI_REVIEW))
}

/// What starting this action does.
pub fn run(id: ActionId) -> Run {
    specs()
        .find(|spec| spec.id == id)
        .map(|spec| spec.run)
        .expect("every action has a spec")
}

/// Why an action cannot be started now, for the UI to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DisabledReason {
    /// It works on one device, and several are selected.
    SeveralDevices,
    /// Actions on a physical Nano X are turned off in the settings.
    DeviceActionsOff,
    NoComposeFile,
    NoTestSuite,
    /// The tests were all unchecked.
    NoTestSelected,
    NoAiModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActionView {
    pub id: ActionId,
    pub label: String,
    pub icon: String,
    pub tooltip: String,
    pub enabled: bool,
    /// Set exactly when the action is not enabled.
    pub disabled_reason: Option<DisabledReason>,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GroupView {
    pub group: Group,
    pub icon: String,
    /// The action of the group that has a button of its own, if it can be offered.
    pub main: Option<ActionView>,
    pub options: Vec<ActionView>,
}

/// The groups with the actions that can be offered for the app, each marked as enabled or not.
pub fn groups(app: &App, selection: &AppSelection, env: &Environment) -> Vec<GroupView> {
    Group::ALL
        .map(|group| {
            let views = specs()
                .filter(|spec| spec.group == group)
                .filter_map(|spec| Some((spec, view(spec, app, selection, env)?)));
            let mut main = None;
            let mut options = Vec::new();
            for (spec, view) in views {
                if spec.main {
                    main = Some(view);
                } else {
                    options.push(view);
                }
            }
            GroupView {
                group,
                icon: group.icon().to_owned(),
                main,
                options,
            }
        })
        .to_vec()
}

/// `None` when the action does not apply to the app at all.
fn view(spec: &Spec, app: &App, selection: &AppSelection, env: &Environment) -> Option<ActionView> {
    if spec
        .language
        .is_some_and(|language| language != app.language)
    {
        return None;
    }
    let prepares_tests = matches!(spec.run, Run::Task(Task::PrepareTestDependencies { .. }));
    if prepares_tests && !prepares_dependencies(app, selection, env) {
        return None;
    }

    let reason = disabled_reason(spec, app, selection, env);
    let as_root = env.user == ContainerUser::Root;
    let label = match spec.id {
        ActionId::OpenTerminal if as_root => "Open Terminal as root",
        _ => spec.label,
    };
    Some(ActionView {
        id: spec.id,
        label: label.to_owned(),
        icon: spec.icon.to_owned(),
        tooltip: spec.tooltip.to_owned(),
        enabled: reason.is_none(),
        disabled_reason: reason,
        running: env.running.contains(&spec.id),
    })
}

fn disabled_reason(
    spec: &Spec,
    app: &App,
    selection: &AppSelection,
    env: &Environment,
) -> Option<DisabledReason> {
    let target = selection.target();
    if spec.one_device && target.is_multiple() {
        return Some(DisabledReason::SeveralDevices);
    }
    match spec.run {
        Run::Task(Task::Device(_)) => {
            let allowed = target
                .single()
                .is_none_or(|device| device_ops::allowed(device, &env.settings.device));
            (!allowed).then_some(DisabledReason::DeviceActionsOff)
        }
        Run::Task(Task::PrepareTestDependencies { .. }) => suite_directory(app, selection)
            .is_none()
            .then_some(DisabledReason::NoTestSuite),
        Run::Task(Task::RunTests(_)) => {
            if suite_directory(app, selection).is_none() {
                Some(DisabledReason::NoTestSuite)
            } else if selection
                .tests
                .as_ref()
                .is_some_and(TestSelection::is_empty)
            {
                Some(DisabledReason::NoTestSelected)
            } else {
                None
            }
        }
        Run::Task(Task::ComposeTerminal) => app
            .compose_file
            .is_none()
            .then_some(DisabledReason::NoComposeFile),
        Run::Shell => env
            .ai_models
            .is_empty()
            .then_some(DisabledReason::NoAiModel),
        _ => None,
    }
}

/// The selected test suite.
fn selected_suite<'a>(app: &'a App, selection: &AppSelection) -> Option<&'a TestSuite> {
    let name = selection.test_suite()?;
    app.test_suites.iter().find(|suite| suite.name == name)
}

/// The directory of the selected test suite, relative to the app folder.
pub fn suite_directory<'a>(app: &'a App, selection: &AppSelection) -> Option<&'a str> {
    selected_suite(app, selection)?.directory.as_deref()
}

/// Whether the selected suite can be prepared: it ships the scripts that do it, or loads apps
/// that the extension knows how to build.
fn prepares_dependencies(app: &App, selection: &AppSelection, env: &Environment) -> bool {
    test_dependencies(app, selection, env)
        .is_some_and(|dependencies| dependencies.can_be_prepared())
}

fn test_dependencies(
    app: &App,
    selection: &AppSelection,
    env: &Environment,
) -> Option<TestDependencies> {
    let suite = selected_suite(app, selection)?;
    Some(TestDependencies {
        suite_directory: suite.directory.clone()?,
        scripts: env.test_helpers,
        sideloaded: suite.sideloaded.clone(),
    })
}

/// What the tasks need to know of the choices and the environment.
pub fn options(app: &App, selection: &AppSelection, env: &Environment) -> Options {
    let suite_directory = suite_directory(app, selection);
    Options {
        build_use_case: selection.build_use_case().and_then(|name| {
            app.build_use_cases
                .iter()
                .find(|use_case| use_case.name == name)
                .cloned()
        }),
        variant: variant(app, selection),
        verbose_tests: env.settings.verbose_tests,
        enforcer_check: selection.enforcer_check().map(str::to_owned),
        tests: suite_directory.map(|directory| test_run(directory, selection, env)),
        test_requirements: Some(env.settings.test_requirements.clone())
            .filter(|requirements| !requirements.trim().is_empty()),
        device: Some(env.settings.device.clone()),
        host: Some(env.host.clone()),
        compose: env.compose_service.clone().map(|service| ComposeRun {
            service,
            user: env.user,
        }),
        test_dependencies: test_dependencies(app, selection, env),
    }
}

fn variant(app: &App, selection: &AppSelection) -> Option<Variant> {
    let variants = app.makefile.as_ref()?.variants.as_ref()?;
    Some(Variant {
        name: variants.name.clone(),
        value: selection.variant()?.to_owned(),
    })
}

/// Without a chosen selection, or before the tests were collected, every test runs.
fn test_run(directory: &str, selection: &AppSelection, env: &Environment) -> TestRun {
    let paths = match (&selection.tests, &env.test_tree) {
        (Some(tests), Some(tree)) => tests.pytest_args_from_app(tree, directory),
        _ => Vec::new(),
    };
    TestRun {
        directory: directory.to_owned(),
        paths,
    }
}

/// Plans a task action for the selected devices. `None` for the other kinds of action.
pub fn plan(
    id: ActionId,
    app: &App,
    selection: &AppSelection,
    env: &Environment,
) -> Option<Result<task::Plan, task::PlanError>> {
    let Run::Task(task) = run(id) else {
        return None;
    };
    let options = options(app, selection, env);
    Some(task::plan_task(task, app, selection.target(), &options))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::device::Device;
    use crate::target::Target;
    use crate::test_support::{c_app, environment, rust_app, selection};
    use crate::test_tree::TestSelection;

    fn views(groups: &[GroupView]) -> Vec<&ActionView> {
        groups
            .iter()
            .flat_map(|group| group.main.iter().chain(&group.options))
            .collect()
    }

    fn labels(views: &[ActionView]) -> Vec<&str> {
        views.iter().map(|view| view.label.as_str()).collect()
    }

    fn group(groups: &[GroupView], group: Group) -> &GroupView {
        groups.iter().find(|view| view.group == group).unwrap()
    }

    fn find(groups: &[GroupView], id: ActionId) -> &ActionView {
        views(groups)
            .into_iter()
            .find(|view| view.id == id)
            .unwrap()
    }

    fn maybe_find(groups: &[GroupView], id: ActionId) -> Option<&ActionView> {
        views(groups).into_iter().find(|view| view.id == id)
    }

    #[test]
    fn every_action_has_exactly_one_spec() {
        let ids: BTreeSet<ActionId> = specs().map(|spec| spec.id).collect();

        assert_eq!(specs().count(), 22);
        assert_eq!(ids.len(), 22);
    }

    #[test]
    fn the_ids_are_the_names_the_old_extension_gave_its_tasks() {
        let names: Vec<String> = [
            ActionId::BuildApp,
            ActionId::CleanAndBuild,
            ActionId::DeleteAppFromDevice,
            ActionId::OpenComposeTerminal,
        ]
        .iter()
        .map(|id| serde_json::to_string(id).unwrap())
        .collect();

        assert_eq!(
            names,
            [
                "\"Build App\"",
                "\"Clean & Build\"",
                "\"Delete App from Device\"",
                "\"Open Compose Terminal\""
            ]
        );
        let back: ActionId = serde_json::from_str("\"Tests on Device\"").unwrap();
        assert_eq!(back, ActionId::TestsOnDevice);
    }

    #[test]
    fn every_group_has_a_main_action_and_the_groups_come_in_the_order_of_the_ui() {
        let app = c_app();
        let groups = groups(&app, &selection(&app), &environment(&app));

        let order: Vec<Group> = groups.iter().map(|view| view.group).collect();
        assert_eq!(order, Group::ALL);
        let mains: Vec<&str> = groups
            .iter()
            .map(|view| view.main.as_ref().unwrap().label.as_str())
            .collect();
        assert_eq!(
            mains,
            [
                "Build App",
                "Run with Emulator",
                "Run Tests",
                "Load on Device",
                "Open Terminal"
            ]
        );
    }

    #[test]
    fn the_other_actions_of_a_group_come_in_the_order_of_the_old_extension() {
        let app = c_app();
        let groups = groups(&app, &selection(&app), &environment(&app));

        assert_eq!(
            labels(&group(&groups, Group::Tools).options),
            [
                "Update Container",
                "Create Container",
                "Open Compose Terminal",
                "Enforcer Checks",
                "AI Review"
            ]
        );
        assert_eq!(
            labels(&group(&groups, Group::Build).options),
            ["Clean & Build", "Clean Target Build", "Clean All Builds"]
        );
    }

    #[test]
    fn some_actions_are_only_for_one_language() {
        let rust = rust_app();
        let groups = groups(&rust, &selection(&rust), &environment(&rust));

        assert_eq!(
            labels(&group(&groups, Group::Build).options),
            ["Update Rust SDK", "Clean All Builds"]
        );
    }

    #[test]
    fn several_devices_turn_off_the_actions_for_one_device() {
        let app = c_app();
        let mut selection = selection(&app);
        let two = Target::new([Device::Stax, Device::Flex]).unwrap();
        selection.set_target(&two, &app);

        let groups = groups(&app, &selection, &environment(&app));

        for id in [
            ActionId::RunEmulator,
            ActionId::KillEmulator,
            ActionId::TestsOnDevice,
            ActionId::CleanTargetBuild,
            ActionId::LoadOnDevice,
            ActionId::DeleteAppFromDevice,
            ActionId::QuickDeviceSetup,
            ActionId::OpenComposeTerminal,
        ] {
            let view = find(&groups, id);
            assert_eq!(
                view.disabled_reason,
                Some(DisabledReason::SeveralDevices),
                "{id:?}"
            );
        }
        for id in [
            ActionId::BuildApp,
            ActionId::RunTests,
            ActionId::OpenTerminal,
            ActionId::EnforcerChecks,
            ActionId::CleanAllBuilds,
        ] {
            assert!(find(&groups, id).enabled, "{id:?}");
        }
    }

    #[test]
    fn the_actions_on_a_device_need_the_setting_that_allows_a_nano_x() {
        let app = c_app();
        let mut selection = selection(&app);
        selection.set_target(&Target::one(Device::NanoX), &app);
        let mut env = environment(&app);

        let off = groups(&app, &selection, &env);
        env.settings.device.allow_nano_x = true;
        let on = groups(&app, &selection, &env);

        for id in [
            ActionId::LoadOnDevice,
            ActionId::DeleteAppFromDevice,
            ActionId::QuickDeviceSetup,
        ] {
            assert_eq!(
                find(&off, id).disabled_reason,
                Some(DisabledReason::DeviceActionsOff)
            );
            assert!(find(&on, id).enabled);
        }
        assert!(find(&off, ActionId::TestsOnDevice).enabled);
    }

    #[test]
    fn what_the_app_lacks_is_the_reason_an_action_is_off() {
        let mut app = c_app();
        app.compose_file = None;
        app.test_suites.clear();
        let selection = selection(&app);
        let mut env = environment(&app);
        env.ai_models.clear();

        let groups = groups(&app, &selection, &env);

        let reason = |id| find(&groups, id).disabled_reason;
        assert_eq!(
            reason(ActionId::OpenComposeTerminal),
            Some(DisabledReason::NoComposeFile)
        );
        assert_eq!(
            reason(ActionId::RunTests),
            Some(DisabledReason::NoTestSuite)
        );
        assert_eq!(
            reason(ActionId::GenerateSnapshots),
            Some(DisabledReason::NoTestSuite)
        );
        assert_eq!(reason(ActionId::AiReview), Some(DisabledReason::NoAiModel));
        assert_eq!(reason(ActionId::BuildApp), None);
    }

    #[test]
    fn unchecking_every_test_turns_off_the_actions_that_run_them() {
        let app = c_app();
        let mut selection = selection(&app);
        selection.tests = Some(TestSelection::default());

        let groups = groups(&app, &selection, &environment(&app));

        for id in [
            ActionId::RunTests,
            ActionId::TestsWithDisplay,
            ActionId::TestsOnDevice,
            ActionId::GenerateSnapshots,
        ] {
            assert_eq!(
                find(&groups, id).disabled_reason,
                Some(DisabledReason::NoTestSelected),
                "{id:?}"
            );
        }
        assert!(find(&groups, ActionId::PrepareTestDependencies).enabled);
    }

    #[test]
    fn the_scripts_that_prepare_test_dependencies_are_only_offered_when_the_suite_has_them() {
        let app = c_app();
        let selection = selection(&app);
        let mut env = environment(&app);

        let with = groups(&app, &selection, &env);
        env.test_helpers = false;
        let without = groups(&app, &selection, &env);

        assert!(find(&with, ActionId::PrepareTestDependencies).enabled);
        assert!(find(&with, ActionId::RebuildTestDependencies).enabled);
        assert!(maybe_find(&without, ActionId::PrepareTestDependencies).is_none());
        assert!(maybe_find(&without, ActionId::RebuildTestDependencies).is_none());
    }

    #[test]
    fn a_suite_without_scripts_is_prepared_when_it_loads_apps_the_extension_can_build() {
        use crate::sideloaded::{Sideloaded, SideloadedApp};

        let mut app = c_app();
        let selection = selection(&app);
        let mut env = environment(&app);
        env.test_helpers = false;
        let name = selection.test_suite().unwrap().to_owned();
        fn loads(app: &mut App, suite: &str, variant: &str) {
            let suite = app
                .test_suites
                .iter_mut()
                .find(|known| known.name == suite)
                .unwrap();
            suite.sideloaded = Some(Sideloaded {
                directory: "libs".to_owned(),
                apps: vec![SideloadedApp {
                    variant: variant.to_owned(),
                    name: variant.to_owned(),
                }],
            });
        }

        loads(&mut app, &name, "unheard_of");
        let unknown = groups(&app, &selection, &env);
        assert!(maybe_find(&unknown, ActionId::PrepareTestDependencies).is_none());

        loads(&mut app, &name, "tezos");
        let known = groups(&app, &selection, &env);
        assert!(find(&known, ActionId::PrepareTestDependencies).enabled);
        assert!(find(&known, ActionId::RebuildTestDependencies).enabled);

        // The plan needs no script: the ignore file, the four Git commands, a build for the
        // selected device and the check that the binary is there.
        let plan = plan(ActionId::PrepareTestDependencies, &app, &selection, &env)
            .unwrap()
            .unwrap();
        assert_eq!(plan.jobs[0].steps.len(), 1 + 4 + 1 + 1);
    }

    #[test]
    fn the_terminal_says_when_it_opens_as_root() {
        let app = c_app();
        let selection = selection(&app);
        let mut env = environment(&app);

        let user = groups(&app, &selection, &env);
        env.user = ContainerUser::Root;
        let root = groups(&app, &selection, &env);

        assert_eq!(find(&user, ActionId::OpenTerminal).label, "Open Terminal");
        assert_eq!(
            find(&root, ActionId::OpenTerminal).label,
            "Open Terminal as root"
        );
    }

    #[test]
    fn the_actions_that_are_running_are_marked() {
        let app = c_app();
        let selection = selection(&app);
        let mut env = environment(&app);
        env.running.insert(ActionId::BuildApp);

        let groups = groups(&app, &selection, &env);

        assert!(find(&groups, ActionId::BuildApp).running);
        assert!(!find(&groups, ActionId::RunTests).running);
    }

    /// The catalog and the planner must not drift apart: what is offered plans, and what lacks
    /// something it needs does not.
    #[test]
    fn an_enabled_action_can_be_planned_and_one_that_lacks_something_cannot() {
        let mut no_compose = c_app();
        no_compose.compose_file = None;
        let mut no_suite = rust_app();
        no_suite.test_suites.clear();
        let cases = [
            (c_app(), Target::one(Device::Stax), false),
            (c_app(), Target::one(Device::NanoX), false),
            (c_app(), Target::one(Device::NanoX), true),
            (
                c_app(),
                Target::new([Device::Stax, Device::Flex]).unwrap(),
                false,
            ),
            (rust_app(), Target::one(Device::Flex), false),
            (no_compose, Target::one(Device::Stax), false),
            (no_suite, Target::one(Device::Stax), false),
        ];

        for (app, target, allow_nano_x) in cases {
            let mut selection = selection(&app);
            selection.set_target(&target, &app);
            let mut env = environment(&app);
            env.settings.device.allow_nano_x = allow_nano_x;

            for view in views(&groups(&app, &selection, &env)) {
                let Some(planned) = plan(view.id, &app, &selection, &env) else {
                    continue;
                };
                let why = format!("{:?} for {:?}", view.id, target.devices());
                assert_eq!(view.enabled, view.disabled_reason.is_none(), "{why}");
                if view.enabled {
                    assert!(planned.is_ok(), "{why}: {planned:?}");
                }
                let lacks = matches!(
                    view.disabled_reason,
                    Some(
                        DisabledReason::DeviceActionsOff
                            | DisabledReason::NoComposeFile
                            | DisabledReason::NoTestSuite
                    )
                );
                if lacks {
                    assert!(planned.is_err(), "{why}: {planned:?}");
                }
            }
        }
    }

    #[test]
    fn only_task_actions_are_planned() {
        let app = c_app();
        let selection = selection(&app);
        let env = environment(&app);

        assert!(plan(ActionId::BuildApp, &app, &selection, &env).is_some());
        assert!(plan(ActionId::UpdateContainer, &app, &selection, &env).is_none());
        assert!(plan(ActionId::AiReview, &app, &selection, &env).is_none());
        assert_eq!(
            run(ActionId::CreateContainer),
            Run::Containers(ContainerAction::Create)
        );
    }

    #[test]
    fn the_options_follow_the_choices() {
        let app = c_app();
        let mut selection = selection(&app);
        selection.set_build_use_case("debug", &app);
        selection.set_variant("LTC", &app);
        selection.set_enforcer_check("icons");
        let mut env = environment(&app);
        env.settings.verbose_tests = true;
        env.settings.test_requirements = "  ".to_owned();

        let options = options(&app, &selection, &env);

        assert_eq!(options.build_use_case.unwrap().name, "debug");
        let variant = options.variant.unwrap();
        assert_eq!(
            (variant.name.as_str(), variant.value.as_str()),
            ("COIN", "LTC")
        );
        assert_eq!(options.enforcer_check.as_deref(), Some("icons"));
        assert!(options.verbose_tests);
        assert_eq!(options.test_requirements, None, "blank is nothing");
        assert_eq!(options.tests.unwrap().directory, "./tests/standalone/");
        assert_eq!(options.compose.unwrap().service, "builder");
    }

    #[test]
    fn the_chosen_tests_become_paths_and_no_choice_means_every_test() {
        let app = c_app();
        let mut selection = selection(&app);
        let mut env = environment(&app);
        let all = options(&app, &selection, &env).tests.unwrap();
        assert!(all.paths.is_empty());

        let tree = env.test_tree.clone().unwrap();
        let mut chosen = TestSelection::default();
        chosen.set(&tree.roots()[1], true);
        selection.tests = Some(chosen);
        let some = options(&app, &selection, &env).tests.unwrap();
        assert_eq!(some.paths, ["tests/standalone/sub"]);

        env.test_tree = None;
        let uncollected = options(&app, &selection, &env).tests.unwrap();
        assert!(uncollected.paths.is_empty());
    }
}
