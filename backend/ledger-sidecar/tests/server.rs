//! The server against an in-memory Docker: the requests, what they answer, and what it tells
//! on its own.

use std::collections::VecDeque;
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
