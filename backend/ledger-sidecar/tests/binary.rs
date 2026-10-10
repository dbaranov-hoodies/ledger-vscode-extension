//! The real binary over its standard input and output.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const BOILERPLATE: &str = include_str!("../../ledger-core/tests/fixtures/boilerplate_c.toml");

#[tokio::test]
async fn the_binary_answers_requests_line_by_line_and_stops_when_its_input_closes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ledger-sidecar"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("the binary starts");
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "initialize",
        "params": {
            "platform": "linux",
            "hostIds": null,
            "display": null,
            "settings": {
                "dockerImage": "ledger-image-that-does-not-exist:latest",
                "dockerRunArgs": "",
                "autoUpdate": false,
                "maxRetries": 1,
                "openAsRoot": false,
                "onboardingPin": "1234",
                "onboardingSeed": "one",
                "scpPrivateKey": null,
                "allowNanoX": false,
                "defaultDevice": null,
                "verboseTests": false,
                "testRequirements": {},
            },
            "saved": null,
            "folders": [{
                "path": "/work/app-boilerplate",
                "uriPath": "/work/app-boilerplate",
                "hostVenv": "/venvs/app",
                "files": [{ "path": "ledger_app.toml", "content": BOILERPLATE }],
            }],
        },
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();

    let mut response = None;
    while response.is_none() {
        let line = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
            .await
            .expect("the binary should answer")
            .unwrap()
            .expect("the binary is still there");
        let message: Value = serde_json::from_str(&line).expect("stdout holds only JSON lines");
        if message["id"] == 7 {
            response = Some(message);
        }
    }

    let snapshot = &response.unwrap()["result"];
    assert_eq!(snapshot["apps"][0]["name"], "app-boilerplate");
    assert_eq!(snapshot["app"]["groups"][0]["main"]["id"], "Build App");

    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("the binary stops when its input closes")
        .unwrap();
    assert!(status.success());
}
