//! The server against an in-memory Docker: the requests, what they answer, and what it tells
//! on its own.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ledger_runtime::fake::FakeDocker;
use ledger_sidecar::Server;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

const BOILERPLATE: &str = include_str!("../../ledger-core/tests/fixtures/boilerplate_c.toml");
const APP: &str = "/work/app-boilerplate";

fn settings() -> Value {
    json!({
        "dockerImage": "img:latest",
        "dockerRunArgs": "",
        "autoUpdate": false,
        "maxRetries": 2,
        "openAsRoot": false,
        "onboardingPin": "1234",
        "onboardingSeed": "one two three",
        "scpPrivateKey": null,
        "allowNanoX": false,
        "defaultDevice": "Stax",
        "verboseTests": false,
        "testRequirements": {},
    })
}

fn folder(path: &str, manifest: &str) -> Value {
    json!({
        "path": path,
        "uriPath": path,
        "hostVenv": "/venvs/app",
        "files": [{ "path": "ledger_app.toml", "content": manifest }],
    })
}

fn initialize_params(folders: Vec<Value>) -> Value {
    json!({
        "platform": "linux",
        "hostIds": { "uid": 1000, "gid": 1000 },
        "display": ":0",
        "settings": settings(),
        "saved": null,
        "folders": folders,
    })
}

fn docker() -> FakeDocker {
    FakeDocker::new()
        .with_make_output("listinfo", "APPNAME=Boilerplate\n")
        .with_make_output("listvariants", "VARIANTS COIN BTC LTC\n")
}

struct Harness {
    server: Server<Arc<FakeDocker>>,
    lines: UnboundedReceiver<String>,
    /// Notifications that came while waiting for something else.
    seen: VecDeque<Value>,
    next_id: u64,
}

type Failure = (i64, String);

impl Harness {
    fn new(docker: FakeDocker) -> Self {
        let (server, lines) = Server::new(Arc::new(docker));
        Self {
            server,
            lines,
            seen: VecDeque::new(),
            next_id: 1,
        }
    }

    async fn next_message(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.recv())
            .await
            .expect("the server should say something")
            .expect("the server is still there");
        serde_json::from_str(&line).expect("every line is JSON")
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.server.handle(&request.to_string()).await;
        loop {
            let message = self.next_message().await;
            if message["id"] == id {
                return match message.get("error") {
                    Some(error) => Err((
                        error["code"].as_i64().unwrap(),
                        error["message"].as_str().unwrap().to_owned(),
                    )),
                    None => Ok(message["result"].clone()),
                };
            }
            self.seen.push_back(message);
        }
    }

    async fn ok(&mut self, method: &str, params: Value) -> Value {
        self.call(method, params)
            .await
            .unwrap_or_else(|failure| panic!("{method} failed: {failure:?}"))
    }

    /// The next notification of this kind that satisfies `wanted`, waiting for it if needed.
    async fn wait_for(&mut self, method: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        loop {
            if let Some(at) = self
                .seen
                .iter()
                .position(|message| message["method"] == method && wanted(&message["params"]))
            {
                return self.seen.remove(at).unwrap()["params"].clone();
            }
            let message = self.next_message().await;
            self.seen.push_back(message);
        }
    }

    /// Everything the server has sent so far that nobody asked for.
    fn notifications(&mut self, method: &str) -> Vec<Value> {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push_back(serde_json::from_str(&line).unwrap());
        }
        self.seen
            .iter()
            .filter(|message| message["method"] == method)
            .map(|message| message["params"].clone())
            .collect()
    }

    async fn initialized(docker: FakeDocker) -> (Self, Value) {
        let mut harness = Self::new(docker);
        let snapshot = harness
            .ok(
                "initialize",
                initialize_params(vec![folder(APP, BOILERPLATE)]),
            )
            .await;
        (harness, snapshot)
    }
}

fn action<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["app"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| {
            std::iter::once(&group["main"]).chain(group["options"].as_array().unwrap())
        })
        .find(|action| action["id"] == id)
        .unwrap_or_else(|| panic!("no action {id}"))
}

#[tokio::test]
async fn initializing_detects_the_apps_and_answers_with_the_snapshot() {
    let (mut harness, snapshot) = Harness::initialized(docker()).await;

    assert_eq!(snapshot["docker"]["running"], true);
    assert_eq!(snapshot["apps"][0]["folder"], APP);
    assert_eq!(snapshot["apps"][0]["name"], "app-boilerplate");
    assert_eq!(snapshot["app"]["language"], "c");
    assert_eq!(snapshot["app"]["devices"]["selected"], json!(["stax"]));
    assert_eq!(
        snapshot["app"]["buildUseCases"]["items"],
        json!(["release", "debug"])
    );
    assert_eq!(action(&snapshot, "Build App")["enabled"], true);
    assert_eq!(snapshot["app"]["groups"][0]["group"], "Build");
    let saved = harness.notifications("saved");
    assert_eq!(saved[0]["saved"]["app"], APP);
}

#[tokio::test]
async fn the_name_and_variants_of_a_c_app_arrive_in_a_later_snapshot() {
    let (mut harness, first) = Harness::initialized(docker()).await;
    assert_eq!(first["app"]["variants"], Value::Null);

    let later = harness
        .wait_for("snapshot", |snapshot| {
            !snapshot["app"]["variants"].is_null()
        })
        .await;

    assert_eq!(later["app"]["variants"]["items"], json!(["BTC", "LTC"]));
    assert_eq!(later["app"]["variants"]["selected"], "BTC");
}

#[tokio::test]
async fn a_workspace_without_apps_still_reports_docker() {
    let mut harness = Harness::new(docker());

    let snapshot = harness.ok("initialize", initialize_params(vec![])).await;

    assert_eq!(snapshot["app"], Value::Null);
    assert_eq!(snapshot["apps"], json!([]));
    assert_eq!(snapshot["docker"]["running"], true);
}

#[tokio::test]
async fn without_docker_the_snapshot_says_so() {
    let (_harness, snapshot) = Harness::initialized(docker().docker_down()).await;

    assert_eq!(snapshot["docker"]["running"], false);
    assert_eq!(snapshot["docker"]["status"], "stopped");
}

#[tokio::test]
async fn a_broken_manifest_is_reported_and_does_not_stop_the_other_apps() {
    let mut harness = Harness::new(docker());
    let folders = vec![
        folder("/work/broken", "[app]\nsdk = \"cobol\"\n"),
        folder(APP, BOILERPLATE),
    ];

    let snapshot = harness.ok("initialize", initialize_params(folders)).await;

    assert_eq!(snapshot["apps"].as_array().unwrap().len(), 1);
    let messages = harness.notifications("message");
    assert_eq!(messages[0]["level"], "error");
    assert!(
        messages[0]["text"]
            .as_str()
            .unwrap()
            .contains("/work/broken")
    );
}

#[tokio::test]
async fn requests_before_initialize_fail() {
    let mut harness = Harness::new(docker());

    let failure = harness
        .call("setDevices", json!({ "devices": ["stax"] }))
        .await
        .unwrap_err();

    assert_eq!(failure.0, -32000);
}

#[tokio::test]
async fn choosing_devices_answers_with_the_snapshot_and_saves_the_choice() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let snapshot = harness
        .ok("setDevices", json!({ "devices": ["stax", "flex"] }))
        .await;

    assert_eq!(
        snapshot["app"]["devices"]["selected"],
        json!(["stax", "flex"])
    );
    assert_eq!(snapshot["app"]["notices"], json!(["severalDevices"]));
    let load = action(&snapshot, "Load on Device");
    assert_eq!(load["enabled"], false);
    assert_eq!(load["disabledReason"], "severalDevices");
    let saved = harness.notifications("saved");
    assert_eq!(
        saved.last().unwrap()["saved"]["apps"][APP]["target"],
        json!({ "devices": ["stax", "flex"] })
    );
}

#[tokio::test]
async fn a_choice_the_app_does_not_offer_is_refused() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let device = harness
        .call("setDevices", json!({ "devices": ["nokia"] }))
        .await
        .unwrap_err();
    let use_case = harness
        .call("setBuildUseCase", json!({ "name": "nope" }))
        .await
        .unwrap_err();
    let ok = harness
        .ok("setBuildUseCase", json!({ "name": "debug" }))
        .await;

    assert_eq!(device.0, -32602);
    assert_eq!(use_case.0, -32602);
    assert_eq!(ok["app"]["buildUseCases"]["selected"], "debug");
}

#[tokio::test]
async fn an_app_is_selected_by_its_folder() {
    let mut harness = Harness::new(docker());
    let folders = vec![folder(APP, BOILERPLATE), folder("/work/other", BOILERPLATE)];
    harness.ok("initialize", initialize_params(folders)).await;

    let snapshot = harness
        .ok("selectApp", json!({ "folder": "/work/other" }))
        .await;
    let failure = harness
        .call("selectApp", json!({ "folder": "/work/nope" }))
        .await
        .unwrap_err();

    assert_eq!(snapshot["app"]["folder"], "/work/other");
    assert_eq!(failure.0, -32000);
}

#[tokio::test]
async fn refreshing_with_the_files_again_keeps_the_choices() {
    let (mut harness, _) = Harness::initialized(docker()).await;
    harness
        .ok("setBuildUseCase", json!({ "name": "debug" }))
        .await;

    let snapshot = harness
        .ok("refresh", json!({ "folders": [folder(APP, BOILERPLATE)] }))
        .await;

    assert_eq!(snapshot["app"]["buildUseCases"]["selected"], "debug");
}

#[tokio::test]
async fn building_creates_the_container_runs_make_and_streams_the_output() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let started = harness.ok("runAction", json!({ "id": "Build App" })).await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(started, Value::Null);
    assert_eq!(finished["id"], "Build App");
    assert_eq!(finished["success"], true, "{finished}");
    let logs = harness.notifications("log");
    let commands: Vec<&str> = logs
        .iter()
        .filter(|log| log["kind"] == "started" && log["source"] == "stax")
        .map(|log| log["command"].as_str().unwrap())
        .collect();
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with("docker run")),
        "the container was created: {commands:?}"
    );
    assert!(
        commands.contains(&"make -C ./ -j"),
        "the build has the stream of the Stax container: {commands:?}"
    );
    let snapshots = harness.notifications("snapshot");
    assert_eq!(
        action(snapshots.first().unwrap(), "Build App")["running"],
        true,
        "told as soon as it starts"
    );
    let last = snapshots.last().unwrap();
    assert_eq!(action(last, "Build App")["running"], false);
    let stax = last["app"]["containers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|container| container["device"]["id"] == "stax")
        .unwrap();
    assert_eq!(stax["state"], "running");
}

#[tokio::test]
async fn a_failing_build_is_reported_with_the_step_that_failed() {
    let docker = docker().fail("docker exec", 1);
    let (mut harness, _) = Harness::initialized(docker).await;

    harness.ok("runAction", json!({ "id": "Build App" })).await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(finished["success"], false);
    assert!(
        finished["message"]
            .as_str()
            .unwrap()
            .contains("step 1 failed")
    );
}

#[tokio::test]
async fn an_action_that_is_already_running_is_refused() {
    let (mut harness, _) = Harness::initialized(docker()).await;
    harness.ok("runAction", json!({ "id": "Build App" })).await;

    let failure = harness
        .call("runAction", json!({ "id": "Build App" }))
        .await
        .unwrap_err();

    assert!(failure.1.contains("already running"), "{failure:?}");
    harness.wait_for("actionFinished", |_| true).await;
}

#[tokio::test]
async fn a_disabled_action_says_why_and_an_interactive_one_is_not_supported_yet() {
    let (mut harness, _) = Harness::initialized(docker()).await;
    harness
        .ok("setDevices", json!({ "devices": ["stax", "flex"] }))
        .await;

    let disabled = harness
        .call("runAction", json!({ "id": "Load on Device" }))
        .await
        .unwrap_err();
    let terminal = harness
        .call("runAction", json!({ "id": "Open Terminal" }))
        .await
        .unwrap_err();
    let shell = harness
        .call("runAction", json!({ "id": "AI Review" }))
        .await
        .unwrap_err();

    assert!(disabled.1.contains("single device"), "{disabled:?}");
    assert!(terminal.1.contains("interactive"), "{terminal:?}");
    assert!(
        shell.1.contains("AI model") || shell.1.contains("extension"),
        "{shell:?}"
    );
}

#[tokio::test]
async fn updating_the_container_pulls_and_creates_it_again() {
    let docker = docker().with_container("app-boilerplate-container-stax", "running");
    let (mut harness, _) = Harness::initialized(docker).await;

    harness
        .ok("runAction", json!({ "id": "Update Container" }))
        .await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(finished["success"], true, "{finished}");
    let logs = harness.notifications("log");
    assert!(
        logs.iter()
            .any(|log| log["kind"] == "started" && log["source"] == "image"),
        "the pull has the stream of the image"
    );
}

#[tokio::test]
async fn bad_lines_and_unknown_methods_get_errors() {
    let mut harness = Harness::new(docker());

    harness.server.handle("this is not json").await;
    let parse = harness.next_message().await;
    let unknown = harness.call("nope", json!({})).await.unwrap_err();
    let invalid = harness
        .call("selectApp", json!({ "wrong": 1 }))
        .await
        .unwrap_err();

    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);
    assert_eq!(unknown.0, -32601);
    assert_eq!(invalid.0, -32602);
}

#[tokio::test]
async fn a_notification_from_the_extension_gets_no_answer() {
    let mut harness = Harness::new(docker());

    harness
        .server
        .handle(&json!({ "jsonrpc": "2.0", "method": "nope" }).to_string())
        .await;

    assert!(harness.notifications("anything").is_empty());
    assert!(harness.lines.try_recv().is_err());
}

#[tokio::test]
async fn what_the_former_extension_saved_is_used_until_something_is_saved() {
    let mut harness = Harness::new(docker());
    let mut former = folder(APP, BOILERPLATE);
    former["legacySettings"] =
        json!({ "selectedDevice": "Flex", "selectedUseCase": "debug", "selectedVariant": "LTC" });

    let snapshot = harness
        .ok("initialize", initialize_params(vec![former.clone()]))
        .await;
    assert_eq!(snapshot["app"]["devices"]["selected"], json!(["flex"]));
    assert_eq!(snapshot["app"]["buildUseCases"]["selected"], "debug");

    // The same folder again, with a different choice made since: the choice wins.
    harness
        .ok("setDevices", json!({ "devices": ["stax"] }))
        .await;
    let again = harness.ok("refresh", json!({ "folders": [former] })).await;
    assert_eq!(again["app"]["devices"]["selected"], json!(["stax"]));
}

#[tokio::test]
async fn what_was_saved_is_given_back_at_initialize() {
    let mut first = Harness::new(docker());
    first
        .ok(
            "initialize",
            initialize_params(vec![folder(APP, BOILERPLATE)]),
        )
        .await;
    first.ok("setDevices", json!({ "devices": ["flex"] })).await;
    let saved = first.notifications("saved").pop().unwrap()["saved"].clone();

    let mut second = Harness::new(docker());
    let mut params = initialize_params(vec![folder(APP, BOILERPLATE)]);
    params["saved"] = saved;
    let snapshot = second.ok("initialize", params).await;

    assert_eq!(snapshot["app"]["devices"]["selected"], json!(["flex"]));
}

#[tokio::test]
async fn the_ai_models_of_the_extension_enable_the_ai_review_and_a_choice_is_kept() {
    let (mut harness, first) = Harness::initialized(docker()).await;
    assert_eq!(action(&first, "AI Review")["disabledReason"], "noAiModel");

    let snapshot = harness
        .ok(
            "setAi",
            json!({ "models": ["gpt-a", "gpt-b"], "selected": "gpt-b" }),
        )
        .await;

    assert_eq!(
        snapshot["app"]["aiModels"]["items"],
        json!(["gpt-a", "gpt-b"])
    );
    assert_eq!(snapshot["app"]["aiModels"]["selected"], "gpt-b");
    assert_eq!(action(&snapshot, "AI Review")["enabled"], true);
    let unknown = harness
        .ok("setAi", json!({ "models": ["gpt-a"], "selected": "gone" }))
        .await;
    assert_eq!(unknown["app"]["aiModels"]["selected"], "gpt-a");
    let none = harness
        .ok("setAi", json!({ "models": [], "selected": null }))
        .await;
    assert_eq!(action(&none, "AI Review")["enabled"], false);
}

const STAX_CONTAINER: &str = "app-boilerplate-container-stax";
const TESTS_OUTPUT: &str = "tests/standalone/test_a.py::test_x[stax]\n\
    tests/standalone/test_a.py::test_y[stax]\n\
    tests/standalone/sub/test_b.py::test_z[stax]\n\n3 tests collected in 0.02s\n";

/// A running container for the selected device, and a pytest that finds three tests.
fn docker_with_tests() -> FakeDocker {
    docker()
        .with_container(STAX_CONTAINER, "running")
        .with_exec_output("--collect-only", TESTS_OUTPUT)
}

fn has_tests(snapshot: &Value) -> bool {
    !snapshot["app"]["tests"]["tree"]
        .as_array()
        .unwrap()
        .is_empty()
}

/// The state of the node with this id, looking through the whole tree.
fn test_state(tree: &Value, id: &str) -> Option<String> {
    tree.as_array()?.iter().find_map(|node| {
        if node["id"] == id {
            node["state"].as_str().map(str::to_owned)
        } else {
            test_state(&node["children"], id)
        }
    })
}

#[tokio::test]
async fn the_tests_are_listed_when_the_container_of_the_device_is_running() {
    let (mut harness, first) = Harness::initialized(docker_with_tests()).await;
    assert_eq!(first["app"]["tests"]["collecting"], true, "told at once");

    let listed = harness.wait_for("snapshot", has_tests).await;

    let tests = &listed["app"]["tests"];
    assert_eq!(tests["collecting"], false);
    assert_eq!(tests["suites"], json!(["standalone", "swap"]));
    assert_eq!(tests["suite"], "standalone");
    assert_eq!(tests["directory"], "./tests/standalone/");
    assert_eq!(tests["tree"][0]["id"], "tests");
    assert_eq!(tests["tree"][0]["kind"], "directory");
    for id in [
        "tests",
        "tests/standalone/test_a.py::test_x",
        "tests/standalone/sub",
    ] {
        assert_eq!(
            test_state(&tests["tree"], id).as_deref(),
            Some("checked"),
            "{id}"
        );
    }
}

#[tokio::test]
async fn without_a_running_container_the_tests_wait_until_they_are_asked_for() {
    let (mut harness, first) =
        Harness::initialized(docker().with_exec_output("--collect-only", TESTS_OUTPUT)).await;
    assert_eq!(first["app"]["tests"]["collecting"], false);
    assert!(!has_tests(&first));

    let asked = harness.ok("collectTests", json!({})).await;
    assert_eq!(asked["app"]["tests"]["collecting"], true);
    let listed = harness.wait_for("snapshot", has_tests).await;

    assert!(has_tests(&listed));
    let stax = listed["app"]["containers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|container| container["device"]["id"] == "stax")
        .unwrap()
        .clone();
    assert_eq!(
        stax["state"], "running",
        "the container was made ready for it"
    );
}

#[tokio::test]
async fn toggling_a_node_changes_the_tests_below_it_and_is_saved() {
    let (mut harness, _) = Harness::initialized(docker_with_tests()).await;
    harness.wait_for("snapshot", has_tests).await;

    let snapshot = harness
        .ok("toggleTest", json!({ "id": "tests/standalone/test_a.py" }))
        .await;

    let tree = &snapshot["app"]["tests"]["tree"];
    assert_eq!(
        test_state(tree, "tests/standalone/test_a.py").as_deref(),
        Some("unchecked")
    );
    assert_eq!(
        test_state(tree, "tests/standalone/test_a.py::test_x").as_deref(),
        Some("unchecked")
    );
    assert_eq!(
        test_state(tree, "tests/standalone").as_deref(),
        Some("partial")
    );
    assert_eq!(
        test_state(tree, "tests/standalone/sub").as_deref(),
        Some("checked")
    );
    let saved = harness.notifications("saved").pop().unwrap();
    assert_eq!(
        saved["saved"]["apps"][APP]["tests"],
        json!(["tests/standalone/sub/test_b.py::test_z"])
    );

    harness
        .ok("toggleTest", json!({ "id": "tests/standalone/test_a.py" }))
        .await;
    let saved = harness.notifications("saved").pop().unwrap();
    assert!(
        saved["saved"]["apps"][APP].get("tests").is_none(),
        "all are chosen again"
    );
}

#[tokio::test]
async fn only_the_chosen_tests_run_and_none_chosen_turns_the_actions_off() {
    let (mut harness, _) = Harness::initialized(docker_with_tests()).await;
    harness.wait_for("snapshot", has_tests).await;

    let none = harness.ok("setTests", json!({ "ids": [] })).await;
    assert_eq!(
        action(&none, "Run Tests")["disabledReason"],
        "noTestSelected"
    );
    let refused = harness
        .call("runAction", json!({ "id": "Run Tests" }))
        .await
        .unwrap_err();
    assert!(refused.1.contains("no test is selected"), "{refused:?}");

    harness
        .ok(
            "setTests",
            json!({ "ids": ["tests/standalone/sub/test_b.py::test_z", "gone.py::x"] }),
        )
        .await;
    harness.ok("runAction", json!({ "id": "Run Tests" })).await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(finished["success"], true, "{finished}");
    let commands: Vec<String> = harness
        .notifications("log")
        .iter()
        .filter(|log| log["kind"] == "started")
        .map(|log| log["command"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with("pytest tests/standalone/sub ")),
        "{commands:?}"
    );
}

#[tokio::test]
async fn the_tests_cannot_be_chosen_before_they_are_listed() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let failure = harness
        .call("toggleTest", json!({ "id": "tests" }))
        .await
        .unwrap_err();

    assert!(failure.1.contains("not listed"), "{failure:?}");
}

#[tokio::test]
async fn changing_the_suite_lists_the_tests_again_and_forgets_the_choice() {
    let (mut harness, _) = Harness::initialized(docker_with_tests()).await;
    harness.wait_for("snapshot", has_tests).await;
    harness
        .ok("toggleTest", json!({ "id": "tests/standalone/test_a.py" }))
        .await;

    let changed = harness.ok("setTestSuite", json!({ "name": "swap" })).await;
    assert_eq!(changed["app"]["tests"]["suite"], "swap");
    assert_eq!(changed["app"]["tests"]["directory"], "./tests/swap/");
    assert_eq!(changed["app"]["tests"]["collecting"], true);
    let listed = harness.wait_for("snapshot", has_tests).await;

    assert_eq!(
        test_state(&listed["app"]["tests"]["tree"], "tests").as_deref(),
        Some("checked")
    );
    let failure = harness
        .call("setTestSuite", json!({ "name": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(failure.0, -32602);
}

#[tokio::test]
async fn a_failing_listing_is_reported_and_ends() {
    let (mut harness, _) =
        Harness::initialized(docker_with_tests().fail("--collect-only", 1)).await;

    let message = harness
        .wait_for("message", |message| message["level"] == "error")
        .await;
    let later = harness
        .wait_for("snapshot", |snapshot| {
            snapshot["app"]["tests"]["collecting"] == false
        })
        .await;

    assert!(
        message["text"]
            .as_str()
            .unwrap()
            .contains("could not be listed")
    );
    assert!(!has_tests(&later));
}

#[tokio::test]
async fn the_chosen_tests_come_back_with_the_saved_state() {
    let (mut first, _) = Harness::initialized(docker_with_tests()).await;
    first.wait_for("snapshot", has_tests).await;
    first
        .ok(
            "toggleTest",
            json!({ "id": "tests/standalone/test_a.py::test_x" }),
        )
        .await;
    let saved = first.notifications("saved").pop().unwrap()["saved"].clone();

    let mut second = Harness::new(docker_with_tests());
    let mut params = initialize_params(vec![folder(APP, BOILERPLATE)]);
    params["saved"] = saved;
    second.ok("initialize", params).await;
    let listed = second.wait_for("snapshot", has_tests).await;

    let tree = &listed["app"]["tests"]["tree"];
    assert_eq!(
        test_state(tree, "tests/standalone/test_a.py::test_x").as_deref(),
        Some("unchecked")
    );
    assert_eq!(
        test_state(tree, "tests/standalone/test_a.py::test_y").as_deref(),
        Some("checked")
    );
}

/// The log notification for a command that started in a container, once it did.
async fn wait_for_command(harness: &mut Harness, text: &'static str) {
    harness
        .wait_for("log", move |log| {
            log["kind"] == "started" && log["command"].as_str().unwrap().contains(text)
        })
        .await;
}

#[tokio::test]
async fn a_running_action_is_stopped_on_request_and_says_it_was_cancelled() {
    let docker = docker_with_tests().blocking("make -C");
    let (mut harness, _) = Harness::initialized(docker).await;
    harness.ok("runAction", json!({ "id": "Build App" })).await;
    wait_for_command(&mut harness, "make -C").await;

    let answer = harness
        .ok("cancelAction", json!({ "id": "Build App" }))
        .await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(answer, Value::Null);
    assert_eq!(finished["id"], "Build App");
    assert_eq!(finished["success"], false);
    assert_eq!(finished["cancelled"], true);
    assert_eq!(finished["message"], Value::Null, "not a failure to report");
    let later = harness
        .wait_for("snapshot", |snapshot| {
            action(snapshot, "Build App")["running"] == false
        })
        .await;
    assert_eq!(action(&later, "Build App")["running"], false);
}

#[tokio::test]
async fn stopping_a_pull_does_not_go_on_to_create_the_containers() {
    let docker = docker().blocking("docker pull");
    let (mut harness, _) = Harness::initialized(docker).await;
    harness
        .ok("runAction", json!({ "id": "Update Container" }))
        .await;
    wait_for_command(&mut harness, "docker pull").await;

    harness
        .ok("cancelAction", json!({ "id": "Update Container" }))
        .await;
    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(finished["cancelled"], true);
    let commands: Vec<String> = harness
        .notifications("log")
        .iter()
        .filter(|log| log["kind"] == "started")
        .map(|log| log["command"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        !commands
            .iter()
            .any(|command| command.starts_with("docker run")),
        "{commands:?}"
    );
}

#[tokio::test]
async fn an_action_that_is_not_running_cannot_be_stopped() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let failure = harness
        .call("cancelAction", json!({ "id": "Build App" }))
        .await
        .unwrap_err();

    assert!(failure.1.contains("not running"), "{failure:?}");
}

#[tokio::test]
async fn an_action_finished_normally_is_not_reported_as_cancelled() {
    let (mut harness, _) = Harness::initialized(docker()).await;
    harness.ok("runAction", json!({ "id": "Build App" })).await;

    let finished = harness.wait_for("actionFinished", |_| true).await;

    assert_eq!(finished["success"], true);
    assert_eq!(finished["cancelled"], false);
}

/// An app folder on disk, named like the app of the other tests, where pytest left these ids as
/// the tests that failed their last run on each device.
fn app_with_failures(name: &str, failed: &[(&str, &[&str])]) -> String {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{name}-{}", std::process::id()))
        .join("app-boilerplate");
    std::fs::create_dir_all(&folder).unwrap();
    for (device, ids) in failed {
        write_failures(&folder, device, ids);
    }
    folder.to_string_lossy().into_owned()
}

fn write_failures(folder: &Path, device: &str, ids: &[&str]) {
    let file = folder.join(format!(".pytest_cache/{device}/v/cache/lastfailed"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let failed: serde_json::Map<String, Value> = ids
        .iter()
        .map(|id| ((*id).to_owned(), json!(true)))
        .collect();
    std::fs::write(file, Value::Object(failed).to_string()).unwrap();
}

/// The server with the three tests listed, for the app in this folder.
async fn listed_in(folder: &str) -> (Harness, Value) {
    let mut harness = Harness::new(docker_with_tests());
    harness
        .ok(
            "initialize",
            initialize_params(vec![self::folder(folder, BOILERPLATE)]),
        )
        .await;
    let listed = harness.wait_for("snapshot", has_tests).await;
    (harness, listed)
}

/// The devices the test with this id failed on, looking through the whole tree.
fn failed_on(tree: &Value, id: &str) -> Option<Value> {
    tree.as_array()?.iter().find_map(|node| {
        if node["id"] == id {
            Some(node["failedOn"].clone())
        } else {
            failed_on(&node["children"], id)
        }
    })
}

const TEST_X: &str = "tests/standalone/test_a.py::test_x";
const TEST_Y: &str = "tests/standalone/test_a.py::test_y";
const TEST_Z: &str = "tests/standalone/sub/test_b.py::test_z";

fn failures_of_two_devices(name: &str) -> String {
    app_with_failures(
        name,
        &[
            (
                "stax",
                &[TEST_X, "tests/standalone/test_a.py::test_y[stax]"],
            ),
            ("flex", &[TEST_Y, TEST_Z]),
        ],
    )
}

#[tokio::test]
async fn the_devices_a_test_failed_on_are_known_when_the_tests_are_listed() {
    let (_harness, listed) = listed_in(&failures_of_two_devices("failures-listed")).await;

    let tree = &listed["app"]["tests"]["tree"];
    assert_eq!(failed_on(tree, TEST_X), Some(json!(["stax"])));
    assert_eq!(failed_on(tree, TEST_Y), Some(json!(["stax", "flex"])));
    assert_eq!(failed_on(tree, TEST_Z), Some(json!(["flex"])));
    assert_eq!(failed_on(tree, "tests"), Some(json!([])), "only tests fail");
}

#[tokio::test]
async fn the_failures_of_a_device_stay_when_another_device_is_selected() {
    let (mut harness, _) = listed_in(&failures_of_two_devices("failures-stay")).await;

    let other = harness
        .ok("setDevices", json!({ "devices": ["nanox"] }))
        .await;

    let tree = &other["app"]["tests"]["tree"];
    assert_eq!(failed_on(tree, TEST_Y), Some(json!(["stax", "flex"])));
}

#[tokio::test]
async fn the_failed_tests_chosen_are_those_of_the_selected_devices() {
    let path = failures_of_two_devices("failures-select");
    let (mut harness, _) = listed_in(&path).await;
    let state = |snapshot: &Value, id: &str| test_state(&snapshot["app"]["tests"]["tree"], id);

    let stax = harness.ok("selectFailedTests", json!({})).await;
    assert_eq!(state(&stax, TEST_X).as_deref(), Some("checked"));
    assert_eq!(state(&stax, TEST_Y).as_deref(), Some("checked"));
    assert_eq!(state(&stax, TEST_Z).as_deref(), Some("unchecked"));
    let saved = harness.notifications("saved").pop().unwrap();
    assert_eq!(
        saved["saved"]["apps"][&path]["tests"],
        json!([TEST_X, TEST_Y])
    );

    harness
        .ok("setDevices", json!({ "devices": ["flex"] }))
        .await;
    let flex = harness.ok("selectFailedTests", json!({})).await;
    assert_eq!(state(&flex, TEST_X).as_deref(), Some("unchecked"));
    assert_eq!(state(&flex, TEST_Z).as_deref(), Some("checked"));

    harness
        .ok("setDevices", json!({ "devices": ["stax", "flex"] }))
        .await;
    let both = harness.ok("selectFailedTests", json!({})).await;
    for id in [TEST_X, TEST_Y, TEST_Z] {
        assert_eq!(state(&both, id).as_deref(), Some("checked"), "{id}");
    }
}

#[tokio::test]
async fn without_failures_on_the_selected_devices_the_choice_stays_and_the_user_is_told() {
    let (mut harness, _) = listed_in(&failures_of_two_devices("failures-none")).await;
    harness
        .ok("setDevices", json!({ "devices": ["nanox"] }))
        .await;
    harness.ok("setTests", json!({ "ids": [TEST_X] })).await;

    let answer = harness.ok("selectFailedTests", json!({})).await;

    let tree = &answer["app"]["tests"]["tree"];
    assert_eq!(test_state(tree, TEST_X).as_deref(), Some("checked"));
    assert_eq!(test_state(tree, TEST_Y).as_deref(), Some("unchecked"));
    let messages = harness.notifications("message");
    assert_eq!(
        messages.last().unwrap(),
        &json!({ "level": "info", "text": "No failed tests on Nano X." })
    );
}

#[tokio::test]
async fn failed_tests_cannot_be_chosen_before_the_tests_are_listed() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let failure = harness
        .call("selectFailedTests", json!({}))
        .await
        .unwrap_err();

    assert!(failure.1.contains("not listed"), "{failure:?}");
}

#[tokio::test]
async fn the_failures_are_read_again_when_tests_have_run() {
    let path = app_with_failures("failures-run", &[]);
    let (mut harness, listed) = listed_in(&path).await;
    assert_eq!(
        failed_on(&listed["app"]["tests"]["tree"], TEST_X),
        Some(json!([]))
    );

    // What pytest writes while the tests run in the container.
    write_failures(Path::new(&path), "stax", &[TEST_X]);
    harness.ok("runAction", json!({ "id": "Run Tests" })).await;
    harness.wait_for("actionFinished", |_| true).await;

    let after = harness
        .wait_for("snapshot", |snapshot| {
            failed_on(&snapshot["app"]["tests"]["tree"], TEST_X) == Some(json!(["stax"]))
        })
        .await;
    assert_eq!(
        failed_on(&after["app"]["tests"]["tree"], TEST_Y),
        Some(json!([]))
    );
}

fn write_file(folder: &str, relative: &str, text: &str) {
    let file = Path::new(folder).join(relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

#[tokio::test]
async fn a_file_a_class_or_a_test_is_located_by_its_path_and_line() {
    let folder = app_with_failures("locate", &[]);
    write_file(
        &folder,
        "tests/standalone/test_a.py",
        "import pytest\n\n\ndef test_x():\n    pass\n\n\ndef test_y():\n    pass\n",
    );
    write_file(
        &folder,
        "tests/standalone/sub/test_b.py",
        "\n\ndef test_z():\n    pass\n",
    );
    let (mut harness, _) = listed_in(&folder).await;

    let located = |answer: &Value| {
        (
            std::path::PathBuf::from(answer["path"].as_str().unwrap()),
            answer["line"].as_u64().unwrap(),
        )
    };
    let in_app = |relative: &str| Path::new(&folder).join(relative);

    let test = harness.ok("locateTest", json!({ "id": TEST_Y })).await;
    assert_eq!(located(&test), (in_app("tests/standalone/test_a.py"), 8));
    let file = harness
        .ok("locateTest", json!({ "id": "tests/standalone/test_a.py" }))
        .await;
    assert_eq!(located(&file), (in_app("tests/standalone/test_a.py"), 1));
    let deeper = harness.ok("locateTest", json!({ "id": TEST_Z })).await;
    assert_eq!(
        located(&deeper),
        (in_app("tests/standalone/sub/test_b.py"), 3)
    );
}

#[tokio::test]
async fn what_cannot_be_opened_says_why() {
    let folder = app_with_failures("locate-refused", &[]);
    let (mut harness, _) = listed_in(&folder).await;

    let directory = harness
        .call("locateTest", json!({ "id": "tests" }))
        .await
        .unwrap_err();
    assert!(directory.1.contains("directory"), "{directory:?}");
    let unknown = harness
        .call("locateTest", json!({ "id": "gone.py::test" }))
        .await
        .unwrap_err();
    assert!(unknown.1.contains("no such node"), "{unknown:?}");
    let missing_file = harness
        .call("locateTest", json!({ "id": TEST_X }))
        .await
        .unwrap_err();
    assert!(missing_file.1.contains("not in"), "{missing_file:?}");
}

#[tokio::test]
async fn a_test_cannot_be_located_before_the_tests_are_listed() {
    let (mut harness, _) = Harness::initialized(docker()).await;

    let failure = harness
        .call("locateTest", json!({ "id": TEST_X }))
        .await
        .unwrap_err();

    assert!(failure.1.contains("not listed"), "{failure:?}");
}
