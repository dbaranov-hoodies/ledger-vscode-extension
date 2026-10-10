//! Everything the UI shows, as one value. The UI renders a snapshot and sends back what the user
//! did; it keeps no state of its own besides how it looks (what is pinned or expanded).

use serde::Serialize;
use ts_rs::TS;

use crate::actions::{self, GroupView};
use crate::app::App;
use crate::container::{ContainerState, Status};
use crate::device::Device;
use crate::device_ops;
use crate::enforcer;
use crate::environment::Environment;
use crate::language::Language;
use crate::selection::AppSelection;
use crate::test_tree::{CheckState, NodeKind, TestNode, TestSelection};
use crate::update::Freshness;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Snapshot {
    pub docker: DockerView,
    /// Every app of the workspace.
    pub apps: Vec<AppSummary>,
    /// The selected app. `None` when the workspace has no app.
    pub app: Option<AppView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DockerView {
    pub running: bool,
    /// How ready the containers of the app are.
    pub status: Status,
    pub image_outdated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppSummary {
    /// The host path of the folder, which identifies the app.
    pub folder: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppView {
    pub folder: String,
    pub name: String,
    pub language: Language,
    pub devices: DevicesView,
    pub build_use_cases: ChoiceView,
    /// `None` when the app has no variants.
    pub variants: Option<ChoiceView>,
    pub enforcer_checks: ChoiceView,
    pub ai_models: ChoiceView,
    pub tests: TestsView,
    /// A shell command to run before the tests, to install what they need.
    pub test_requirements: String,
    pub containers: Vec<ContainerView>,
    pub groups: Vec<GroupView>,
    pub notices: Vec<Notice>,
}

/// A list to choose from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChoiceView {
    pub items: Vec<String>,
    pub selected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevicesView {
    /// The devices the app supports.
    pub available: Vec<DeviceView>,
    /// The ids of the selected devices.
    pub selected: Vec<String>,
    /// Every device of an app that has several is selected.
    pub all: bool,
    /// Text for the status bar: `All`, or the names of the selected devices.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContainerView {
    pub device: DeviceView,
    pub container_name: String,
    pub state: ContainerState,
    /// A run is active in it.
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestsView {
    pub suites: Vec<String>,
    pub suite: Option<String>,
    /// Empty until the tests were collected.
    pub tree: Vec<TestNodeView>,
    pub collecting: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestNodeView {
    /// The pytest node id.
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    pub state: CheckState,
    pub children: Vec<TestNodeView>,
}

/// Something to tell the user about the current choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Notice {
    /// Several devices are selected, so the actions for one device are off.
    SeveralDevices,
    /// The selected Nano X needs a special development device for the actions on a device.
    DeviceActionsUnsupported,
}

pub fn build(workspace: &Workspace, env: &Environment) -> Snapshot {
    Snapshot {
        docker: DockerView {
            running: env.containers.docker_available,
            status: env.containers.status(),
            image_outdated: env.containers.freshness == Freshness::Outdated,
        },
        apps: workspace
            .apps()
            .map(|(app, _)| AppSummary {
                folder: folder_text(app),
                name: app.folder_name(),
            })
            .collect(),
        app: workspace
            .current()
            .map(|(app, selection)| app_view(app, selection, env)),
    }
}

fn folder_text(app: &App) -> String {
    app.folder.to_string_lossy().into_owned()
}

fn app_view(app: &App, selection: &AppSelection, env: &Environment) -> AppView {
    AppView {
        folder: folder_text(app),
        name: app.folder_name(),
        language: app.language,
        devices: devices_view(app, selection),
        build_use_cases: ChoiceView {
            items: app
                .build_use_cases
                .iter()
                .map(|use_case| use_case.name.clone())
                .collect(),
            selected: selection.build_use_case().map(str::to_owned),
        },
        variants: app
            .makefile
            .as_ref()
            .and_then(|makefile| makefile.variants.as_ref())
            .map(|variants| ChoiceView {
                items: variants.values.clone(),
                selected: selection.variant().map(str::to_owned),
            }),
        enforcer_checks: ChoiceView {
            items: std::iter::once(enforcer::ALL)
                .chain(enforcer::CHECKS)
                .map(str::to_owned)
                .collect(),
            selected: Some(
                selection
                    .enforcer_check()
                    .unwrap_or(enforcer::ALL)
                    .to_owned(),
            ),
        },
        ai_models: ChoiceView {
            items: env.ai_models.clone(),
            selected: env.ai_model.clone(),
        },
        tests: tests_view(app, selection, env),
        test_requirements: env.settings.test_requirements.clone(),
        containers: containers_view(app, env),
        groups: actions::groups(app, selection, env),
        notices: notices(selection, env),
    }
}

fn device_view(device: Device) -> DeviceView {
    let spec = device.spec();
    DeviceView {
        id: spec.name.to_owned(),
        name: spec.display_name.to_owned(),
    }
}

fn devices_view(app: &App, selection: &AppSelection) -> DevicesView {
    let target = selection.target();
    DevicesView {
        available: app.devices.iter().copied().map(device_view).collect(),
        selected: target
            .devices()
            .iter()
            .map(|device| device.spec().name.to_owned())
            .collect(),
        all: target.is_all(&app.devices),
        label: target.label(&app.devices),
    }
}

fn containers_view(app: &App, env: &Environment) -> Vec<ContainerView> {
    app.devices
        .iter()
        .map(|&device| ContainerView {
            device: device_view(device),
            container_name: app.container_name(device),
            state: env.containers.state_of(device),
            busy: env
                .containers
                .containers
                .iter()
                .any(|container| container.device == device && container.busy),
        })
        .collect()
}

fn tests_view(app: &App, selection: &AppSelection, env: &Environment) -> TestsView {
    // Until some tests are chosen, all of them are.
    let tree = env.test_tree.as_ref().map(|tree| {
        let chosen = selection
            .tests
            .clone()
            .unwrap_or_else(|| TestSelection::all(tree));
        tree.roots()
            .iter()
            .map(|node| node_view(node, &chosen))
            .collect()
    });
    TestsView {
        suites: app
            .test_suites
            .iter()
            .map(|suite| suite.name.clone())
            .collect(),
        suite: selection.test_suite().map(str::to_owned),
        tree: tree.unwrap_or_default(),
        collecting: env.collecting_tests,
        verbose: env.settings.verbose_tests,
    }
}

fn node_view(node: &TestNode, chosen: &TestSelection) -> TestNodeView {
    TestNodeView {
        id: node.id.clone(),
        name: node.name.clone(),
        kind: node.kind,
        state: chosen.state(node),
        children: node
            .children
            .iter()
            .map(|child| node_view(child, chosen))
            .collect(),
    }
}

fn notices(selection: &AppSelection, env: &Environment) -> Vec<Notice> {
    let target = selection.target();
    if target.is_multiple() {
        return vec![Notice::SeveralDevices];
    }
    let unsupported = target
        .single()
        .is_some_and(|device| !device_ops::allowed(device, &env.settings.device));
    if unsupported {
        vec![Notice::DeviceActionsUnsupported]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ActionId;
    use crate::container::ObservedContainer;
    use crate::target::Target;
    use crate::test_support::{c_app, environment, rust_app};
    use crate::workspace::SavedWorkspace;

    fn workspace(apps: Vec<App>) -> Workspace {
        let mut workspace = Workspace::default();
        workspace.refresh(apps, &SavedWorkspace::default(), Some(Device::Stax));
        workspace
    }

    fn current(snapshot: &Snapshot) -> &AppView {
        snapshot.app.as_ref().unwrap()
    }

    #[test]
    fn a_workspace_without_apps_has_a_snapshot_without_one() {
        let env = environment(&c_app());

        let snapshot = build(&workspace(Vec::new()), &env);

        assert!(snapshot.apps.is_empty());
        assert_eq!(snapshot.app, None);
        assert!(snapshot.docker.running, "docker is not about the apps");
    }

    #[test]
    fn the_snapshot_lists_the_apps_and_describes_the_selected_one() {
        let apps = vec![c_app(), rust_app()];
        let env = environment(&apps[0]);

        let snapshot = build(&workspace(apps), &env);

        let names: Vec<&str> = snapshot.apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["app-boilerplate", "app-rust"]);
        assert_eq!(snapshot.apps[0].folder, "/work/app-boilerplate");
        let app = current(&snapshot);
        assert_eq!(app.folder, "/work/app-boilerplate");
        assert_eq!(app.language, Language::C);
        assert_eq!(app.build_use_cases.items, ["release", "debug"]);
        assert_eq!(app.build_use_cases.selected.as_deref(), Some("release"));
        let variants = app.variants.as_ref().unwrap();
        assert_eq!(variants.items, ["BTC", "LTC"]);
        assert_eq!(variants.selected.as_deref(), Some("BTC"));
        assert_eq!(app.enforcer_checks.items[0], "All");
        assert_eq!(app.enforcer_checks.selected.as_deref(), Some("All"));
        assert_eq!(app.ai_models.items, ["gpt"]);
    }

    #[test]
    fn an_app_without_variants_has_none() {
        let rust = rust_app();
        let env = environment(&rust);

        let snapshot = build(&workspace(vec![rust]), &env);

        assert_eq!(current(&snapshot).variants, None);
    }

    #[test]
    fn the_devices_say_what_is_available_and_what_is_selected() {
        let app = c_app();
        let env = environment(&app);
        let mut workspace = workspace(vec![app]);
        let one = build(&workspace, &env);
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_target(&Target::all(&app.devices).unwrap(), app);
        let all = build(&workspace, &env);

        let devices = &current(&one).devices;
        assert_eq!(devices.available.len(), 5);
        assert_eq!(devices.available[1].id, "nanosp");
        assert_eq!(devices.available[1].name, "Nano S Plus");
        assert_eq!(devices.selected, ["stax"]);
        assert!(!devices.all);
        assert_eq!(devices.label, "Stax");
        assert!(current(&all).devices.all);
        assert_eq!(current(&all).devices.label, "All");
    }

    #[test]
    fn docker_and_the_containers_are_described() {
        let app = c_app();
        let mut env = environment(&app);
        env.containers.freshness = Freshness::Outdated;
        env.containers.containers = vec![
            ObservedContainer {
                device: Device::Stax,
                state: ContainerState::Running,
                image_id: None,
                busy: true,
            },
            ObservedContainer {
                device: Device::Flex,
                state: ContainerState::Stopped,
                image_id: None,
                busy: false,
            },
        ];

        let snapshot = build(&workspace(vec![app]), &env);

        assert!(snapshot.docker.running);
        assert_eq!(snapshot.docker.status, Status::Stopped);
        assert!(snapshot.docker.image_outdated);
        let containers = &current(&snapshot).containers;
        assert_eq!(containers.len(), 5, "one per device of the app");
        let stax = containers.iter().find(|c| c.device.id == "stax").unwrap();
        assert_eq!(stax.container_name, "app-boilerplate-container-stax");
        assert_eq!(stax.state, ContainerState::Running);
        assert!(stax.busy);
        let flex = containers.iter().find(|c| c.device.id == "flex").unwrap();
        assert_eq!(flex.state, ContainerState::Stopped);
        let nano = containers.iter().find(|c| c.device.id == "nanox").unwrap();
        assert_eq!(nano.state, ContainerState::Missing);
    }

    #[test]
    fn without_docker_nothing_is_running() {
        let app = c_app();
        let mut env = environment(&app);
        env.containers.docker_available = false;
        env.containers.containers.clear();

        let snapshot = build(&workspace(vec![app]), &env);

        assert!(!snapshot.docker.running);
        assert_eq!(snapshot.docker.status, Status::Stopped);
    }

    #[test]
    fn the_tests_are_a_tree_that_is_all_checked_until_some_are_chosen() {
        let app = c_app();
        let env = environment(&app);
        let mut workspace = workspace(vec![app]);
        let all = build(&workspace, &env);
        let tree = env.test_tree.as_ref().unwrap();
        let (_, selection) = workspace.current_mut().unwrap();
        let mut chosen = TestSelection::default();
        chosen.set(&tree.roots()[0].children[0], true);
        selection.tests = Some(chosen);
        let some = build(&workspace, &env);

        let tests = &current(&all).tests;
        assert_eq!(tests.suites, ["standalone", "swap"]);
        assert_eq!(tests.suite.as_deref(), Some("standalone"));
        assert_eq!(tests.tree.len(), 2);
        assert!(
            tests
                .tree
                .iter()
                .all(|node| node.state == CheckState::Checked)
        );
        let partial = &current(&some).tests.tree;
        assert_eq!(partial[0].id, "test_a.py");
        assert_eq!(partial[0].kind, NodeKind::File);
        assert_eq!(partial[0].state, CheckState::Partial);
        assert_eq!(partial[0].children[0].state, CheckState::Checked);
        assert_eq!(partial[0].children[1].state, CheckState::Unchecked);
        assert_eq!(partial[1].state, CheckState::Unchecked);
    }

    #[test]
    fn before_the_tests_are_collected_there_is_no_tree() {
        let app = c_app();
        let mut env = environment(&app);
        env.test_tree = None;
        env.collecting_tests = true;

        let snapshot = build(&workspace(vec![app]), &env);

        assert!(current(&snapshot).tests.tree.is_empty());
        assert!(current(&snapshot).tests.collecting);
    }

    #[test]
    fn the_notices_explain_what_the_choices_turn_off() {
        let app = c_app();
        let env = environment(&app);
        let mut workspace = workspace(vec![app]);
        let none = build(&workspace, &env);
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_target(&Target::all(&app.devices).unwrap(), app);
        let several = build(&workspace, &env);
        let (app, selection) = workspace.current_mut().unwrap();
        selection.set_target(&Target::one(Device::NanoX), app);
        let nano_x = build(&workspace, &env);

        assert!(current(&none).notices.is_empty());
        assert_eq!(current(&several).notices, [Notice::SeveralDevices]);
        assert_eq!(current(&nano_x).notices, [Notice::DeviceActionsUnsupported]);
    }

    #[test]
    fn the_snapshot_reaches_the_ui_as_camel_case_json_with_the_old_action_names() {
        let app = c_app();
        let mut env = environment(&app);
        env.running.insert(ActionId::BuildApp);

        let snapshot = build(&workspace(vec![app]), &env);

        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["docker"]["imageOutdated"], false);
        assert_eq!(json["docker"]["status"], "running");
        assert_eq!(json["app"]["devices"]["selected"][0], "stax");
        assert_eq!(
            json["app"]["containers"][0]["containerName"],
            "app-boilerplate-container-nanox"
        );
        let build_group = &json["app"]["groups"][0];
        assert_eq!(build_group["group"], "Build");
        assert_eq!(build_group["main"]["id"], "Build App");
        assert_eq!(build_group["main"]["running"], true);
        assert_eq!(
            build_group["main"]["disabledReason"],
            serde_json::Value::Null
        );
        assert_eq!(json["app"]["tests"]["tree"][0]["state"], "checked");
    }
}
