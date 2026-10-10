//! Needs Docker and the Ledger builder image. Run with:
//! `cargo test -p ledger-runtime -- --ignored`
//! Set `LEDGER_TEST_IMAGE` to use another image that has `sh` and `bash`.

use std::time::Duration;

use ledger_core::Command;
use ledger_core::docker::{self, ContainerUser};
use ledger_runtime::{CommandRunner, SystemRunner, new_run_id};

const DEFAULT_IMAGE: &str = "ghcr.io/ledgerhq/ledger-app-builder/ledger-app-dev-tools:latest";

/// Stops the throwaway container when the test ends, even when it fails.
struct StopOnDrop(String);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["stop", "-t", "1", &self.0])
            .output();
    }
}

/// How many processes of the container have a command line starting with `prefix`.
async fn running(container: &str, prefix: &str) -> usize {
    let script = format!(
        "for c in /proc/[0-9]*/cmdline; do tr '\\0' ' ' 2>/dev/null < \"$c\"; echo; done | grep -c '^{prefix}'"
    );
    let command = Command::new("docker").args(["exec", container, "bash", "-c", &script]);
    let output = SystemRunner.run(&command, None).await.expect("docker runs");
    output.stdout.trim().parse().unwrap_or(0)
}

#[tokio::test]
#[ignore = "needs Docker and the Ledger builder image"]
async fn a_run_survives_its_client_and_is_stopped_from_inside() {
    let image = std::env::var("LEDGER_TEST_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_owned());
    let container = format!("ledger-runtime-test-{}", std::process::id());
    let start = Command::new("docker").args([
        "run",
        "-d",
        "--rm",
        "--init",
        "--name",
        &container,
        "--entrypoint",
        "sleep",
        &image,
        "600",
    ]);
    assert!(SystemRunner.run(&start, None).await.unwrap().success);
    let _stop = StopOnDrop(container.clone());

    let run_id = new_run_id();
    let tagged = docker::exec_tagged(
        &container,
        ContainerUser::Root,
        &run_id,
        false,
        &[
            "sh".to_owned(),
            "-c".to_owned(),
            "sleep 555 & wait".to_owned(),
        ],
    );
    let mut client = tokio::process::Command::new(&tagged.program)
        .args(&tagged.args)
        .kill_on_drop(true)
        .spawn()
        .expect("docker exec starts");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        running(&container, "sleep 555").await,
        1,
        "the run did not start"
    );

    client
        .kill()
        .await
        .expect("the client dies, like a closed terminal");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        running(&container, "sleep 555").await,
        1,
        "Docker is expected to leave the process running after its client died"
    );

    let kill = docker::kill_run(&container, &run_id);
    assert!(SystemRunner.run(&kill, None).await.unwrap().success);
    assert_eq!(
        running(&container, "sleep 555").await,
        0,
        "the run was not stopped"
    );
}
