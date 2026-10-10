//! The `docker` and `xhost` command lines of the container lifecycle, and the parsing of what
//! they print.

use std::time::Duration;

use crate::command::Command;
use crate::platform::Platform;

/// Looking at the local Docker state is quick, so a slow answer means something is wrong.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(10);
/// Asking the registry crosses the network.
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(20);

/// Environment variable that tags every process of one run, so the run can be found and killed.
pub const RUN_ID_ENV: &str = "LEDGER_RUN_ID";

/// Identifies one run, such as an emulator session or a test run. Letters, digits, `-` and `_`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunId(String);

impl RunId {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let valid = !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        valid.then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The user a container runs commands as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerUser {
    Root,
    Host { uid: u32, gid: u32 },
}

impl ContainerUser {
    /// Root when asked to or on Windows. Otherwise the host user, or `1000:1000` when the host
    /// ids are unknown.
    pub fn resolve(open_as_root: bool, platform: Platform, host_ids: Option<(u32, u32)>) -> Self {
        if open_as_root || platform == Platform::Windows {
            return Self::Root;
        }
        let (uid, gid) = host_ids.unwrap_or((1000, 1000));
        Self::Host { uid, gid }
    }

    pub fn args(self) -> [String; 2] {
        let user = match self {
            Self::Root => "0".to_owned(),
            Self::Host { uid, gid } => format!("{uid}:{gid}"),
        };
        ["--user".to_owned(), user]
    }
}

fn docker<const N: usize>(args: [&str; N]) -> Command {
    Command::new("docker").args(args)
}

pub fn info() -> Command {
    docker(["info"]).timeout(LOCAL_TIMEOUT)
}

/// Prints the Docker status of a container and the id of its image, such as
/// `running sha256:…`. Fails when there is no such container.
pub fn container_status(container: &str) -> Command {
    docker(["inspect", "-f", "{{.State.Status}} {{.Image}}", container]).timeout(LOCAL_TIMEOUT)
}

/// Prints the id of the local image, and fails when it is not there.
pub fn image_id(image: &str) -> Command {
    docker(["image", "inspect", "-f", "{{.Id}}", image]).timeout(LOCAL_TIMEOUT)
}

pub fn local_repo_digest(image: &str) -> Command {
    docker(["inspect", "--format={{index .RepoDigests 0}}", image]).timeout(LOCAL_TIMEOUT)
}

/// Reads the manifest of the image from the registry, without pulling anything.
pub fn remote_manifest(image: &str) -> Command {
    docker([
        "buildx",
        "imagetools",
        "inspect",
        image,
        "--format",
        "{{json .}}",
    ])
    .timeout(REGISTRY_TIMEOUT)
}

pub fn pull(image: &str) -> Command {
    docker(["pull", image])
}

pub fn start(container: &str) -> Command {
    docker(["start", container])
}

pub fn stop(container: &str) -> Command {
    docker(["stop", container])
}

pub fn remove(container: &str) -> Command {
    docker(["rm", container])
}

/// Lets the container show windows on the host's X server. Its failure does not matter.
pub fn allow_x11(platform: Platform) -> Option<Command> {
    match platform {
        Platform::Linux | Platform::MacOs => Some(Command::new("xhost").arg("+")),
        Platform::Windows => None,
    }
}

/// The path of the app folder as `docker -v` expects it, from the path of a VS Code URI.
pub fn host_mount_path(uri_path: &str, platform: Platform) -> String {
    match (platform, uri_path.strip_prefix('/')) {
        (Platform::Windows, Some(path)) => path.to_owned(),
        _ => uri_path.to_owned(),
    }
}

pub struct CreateContainer<'a> {
    pub name: &'a str,
    pub image: &'a str,
    /// Host path of the app folder, mounted at `/app`.
    pub app_dir: &'a str,
    pub user: ContainerUser,
    /// The user's own `docker run` arguments, already split.
    pub extra_args: &'a [String],
    pub platform: Platform,
    /// The host's `DISPLAY`, used on Linux.
    pub display: Option<&'a str>,
}

/// Creates and starts a detached container with everything the Ledger tools need.
pub fn create_container(spec: &CreateContainer) -> Command {
    let mut command = docker(["run", "--init"])
        .args(spec.user.args())
        .arg("--privileged");

    match spec.platform {
        Platform::Linux => {
            if let Some(display) = spec.display {
                command = command.args(["-e".to_owned(), format!("DISPLAY={display}")]);
            }
            command = command.args([
                "-v",
                "/dev/bus/usb:/dev/bus/usb",
                "-v",
                "/tmp/.X11-unix:/tmp/.X11-unix",
            ]);
        }
        Platform::MacOs => {
            command = command.args([
                "-e",
                "DISPLAY=host.docker.internal:0",
                "-v",
                "/tmp/.X11-unix:/tmp/.X11-unix",
            ]);
        }
        Platform::Windows => {
            command = command.args(["-e", "DISPLAY=host.docker.internal:0"]);
        }
    }

    command
        .args(["-v".to_owned(), format!("{}:/app", spec.app_dir)])
        .args(spec.extra_args.iter().cloned())
        .args([
            "-v",
            "ledger-pip-cache:/pip-cache",
            "-e",
            "PIP_CACHE_DIR=/pip-cache",
            "-t",
            "-d",
            "--name",
            spec.name,
            spec.image,
        ])
}

/// Runs a command in a container, tagging its processes with the run id so that
/// [`kill_run`] can find them. `tty` is for commands the user types into.
pub fn exec_tagged(
    container: &str,
    user: ContainerUser,
    run_id: &RunId,
    tty: bool,
    command: &[String],
) -> Command {
    Command::new("docker")
        .arg("exec")
        .arg(if tty { "-it" } else { "-i" })
        .args(user.args())
        .arg("-e")
        .arg(format!("{RUN_ID_ENV}={}", run_id.as_str()))
        .arg(container)
        .args(command.iter().cloned())
}

/// Finds the processes tagged with the run id wherever they are in the container, and stops
/// them: first politely, then by force. The id is an argument, never part of the script.
const KILL_RUN_SCRIPT: &str = r##"
id="LEDGER_RUN_ID=$1"
pids=""
for environ in /proc/[0-9]*/environ; do
  if tr '\0' '\n' 2>/dev/null < "$environ" | grep -qxF "$id"; then
    pid=${environ#/proc/}
    pids="$pids ${pid%/environ}"
  fi
done
[ -n "$pids" ] || exit 0
kill $pids 2>/dev/null
for _ in 1 2 3 4 5 6 7 8 9 10; do
  sleep 0.2
  alive=""
  for pid in $pids; do kill -0 "$pid" 2>/dev/null && alive="$alive $pid"; done
  [ -n "$alive" ] || exit 0
  pids=$alive
done
kill -9 $pids 2>/dev/null
exit 0
"##;

/// Stops every process of a run in a container. Docker has no way to do this when the client
/// that started the run is gone, so it is done from inside, as root.
pub fn kill_run(container: &str, run_id: &RunId) -> Command {
    docker([
        "exec",
        "--user",
        "0",
        container,
        "bash",
        "-c",
        KILL_RUN_SCRIPT,
        "_",
        run_id.as_str(),
    ])
}

/// Reads the output of `docker inspect --format={{index .RepoDigests 0}}`: `repo@sha256:…`.
pub fn parse_local_digest(output: &str) -> Option<&str> {
    let (_repository, digest) = output.trim().split_once('@')?;
    (!digest.is_empty()).then_some(digest)
}

/// Reads the output of `docker buildx imagetools inspect --format {{json .}}`.
pub fn parse_remote_digest(output: &str) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_str(output).ok()?;
    manifest
        .pointer("/manifest/digest")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_spec<'a>(extra: &'a [String], display: Option<&'a str>) -> CreateContainer<'a> {
        CreateContainer {
            name: "app-x-container-stax",
            image: "img:latest",
            app_dir: "/home/u/app-x",
            user: ContainerUser::Host {
                uid: 1000,
                gid: 1001,
            },
            extra_args: extra,
            platform: Platform::Linux,
            display,
        }
    }

    #[test]
    fn creates_a_linux_container_with_usb_x11_and_the_app_mounted() {
        let extra = ["--network".to_owned(), "host".to_owned()];
        let command = create_container(&linux_spec(&extra, Some(":0")));

        assert_eq!(command.program, "docker");
        assert_eq!(
            command.args,
            [
                "run",
                "--init",
                "--user",
                "1000:1001",
                "--privileged",
                "-e",
                "DISPLAY=:0",
                "-v",
                "/dev/bus/usb:/dev/bus/usb",
                "-v",
                "/tmp/.X11-unix:/tmp/.X11-unix",
                "-v",
                "/home/u/app-x:/app",
                "--network",
                "host",
                "-v",
                "ledger-pip-cache:/pip-cache",
                "-e",
                "PIP_CACHE_DIR=/pip-cache",
                "-t",
                "-d",
                "--name",
                "app-x-container-stax",
                "img:latest",
            ]
        );
    }

    #[test]
    fn a_missing_display_is_left_out_on_linux() {
        let command = create_container(&linux_spec(&[], None));

        assert!(!command.args.iter().any(|arg| arg.starts_with("DISPLAY=")));
    }

    #[test]
    fn mac_containers_use_the_docker_desktop_display_and_no_usb() {
        let spec = CreateContainer {
            platform: Platform::MacOs,
            ..linux_spec(&[], Some(":0"))
        };
        let args = create_container(&spec).args;

        assert!(args.contains(&"DISPLAY=host.docker.internal:0".to_owned()));
        assert!(args.contains(&"/tmp/.X11-unix:/tmp/.X11-unix".to_owned()));
        assert!(!args.contains(&"/dev/bus/usb:/dev/bus/usb".to_owned()));
    }

    #[test]
    fn windows_containers_have_neither_usb_nor_x11_mounts() {
        let spec = CreateContainer {
            platform: Platform::Windows,
            user: ContainerUser::Root,
            app_dir: "c:/Users/u/app-x",
            ..linux_spec(&[], None)
        };
        let args = create_container(&spec).args;

        assert!(args.windows(2).any(|pair| pair == ["--user", "0"]));
        assert!(args.contains(&"DISPLAY=host.docker.internal:0".to_owned()));
        assert!(args.contains(&"c:/Users/u/app-x:/app".to_owned()));
        assert!(!args.iter().any(|arg| arg.contains("X11-unix")));
        assert!(!args.iter().any(|arg| arg.contains("/dev/bus/usb")));
    }

    #[test]
    fn x11_is_only_opened_on_linux_and_mac() {
        assert_eq!(
            allow_x11(Platform::Linux),
            Some(Command::new("xhost").arg("+"))
        );
        assert!(allow_x11(Platform::MacOs).is_some());
        assert_eq!(allow_x11(Platform::Windows), None);
    }

    #[test]
    fn windows_mount_paths_lose_the_leading_slash_of_the_uri_path() {
        assert_eq!(
            host_mount_path("/c:/Users/u/app", Platform::Windows),
            "c:/Users/u/app"
        );
        assert_eq!(
            host_mount_path("/home/u/app", Platform::Linux),
            "/home/u/app"
        );
    }

    #[test]
    fn the_container_user_follows_the_host_unless_root_is_wanted() {
        let ids = Some((501, 20));

        assert_eq!(
            ContainerUser::resolve(false, Platform::Linux, ids),
            ContainerUser::Host { uid: 501, gid: 20 }
        );
        assert_eq!(
            ContainerUser::resolve(false, Platform::Linux, None),
            ContainerUser::Host {
                uid: 1000,
                gid: 1000
            }
        );
        assert_eq!(
            ContainerUser::resolve(true, Platform::Linux, ids),
            ContainerUser::Root
        );
        assert_eq!(
            ContainerUser::resolve(false, Platform::Windows, ids),
            ContainerUser::Root
        );
        assert_eq!(ContainerUser::Root.args(), ["--user", "0"]);
    }

    /// Runs the real cleanup script against real processes, on this machine, not in a container.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_cleanup_script_stops_the_processes_of_one_run_and_only_those() {
        use std::process::{Command as Process, Stdio};
        use std::sync::mpsc;
        use std::time::Duration;

        let id = format!("cleanup-test-{}", std::process::id());
        let sleeper = |tag: &str| {
            Process::new("sleep")
                .arg("60")
                .env(RUN_ID_ENV, tag)
                .stdin(Stdio::null())
                .spawn()
                .expect("sleep starts")
        };
        let mut tagged = sleeper(&id);
        let mut other = sleeper("another-run");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(tagged.wait());
        });

        let status = Process::new("bash")
            .args(["-c", KILL_RUN_SCRIPT, "_", &id])
            .status()
            .expect("bash runs");

        assert!(status.success());
        let ended = receiver.recv_timeout(Duration::from_secs(5));
        assert!(ended.is_ok(), "the tagged process should have been stopped");
        assert!(
            other.try_wait().unwrap().is_none(),
            "a process of another run must be left alone"
        );
        other.kill().unwrap();
        other.wait().unwrap();
    }

    #[test]
    fn run_ids_only_accept_safe_characters() {
        assert!(RunId::new("1a2b-3c_4d").is_some());
        assert!(RunId::new("").is_none());
        assert!(RunId::new("a b").is_none());
        assert!(RunId::new("x;rm").is_none());
    }

    #[test]
    fn exec_tags_the_run_and_only_allocates_a_tty_when_asked() {
        let run_id = RunId::new("abc").unwrap();
        let command = vec!["speculos".to_owned(), "app.elf".to_owned()];

        let plain = exec_tagged("c", ContainerUser::Root, &run_id, false, &command);
        let tty = exec_tagged("c", ContainerUser::Root, &run_id, true, &command);

        assert_eq!(
            plain.args,
            [
                "exec",
                "-i",
                "--user",
                "0",
                "-e",
                "LEDGER_RUN_ID=abc",
                "c",
                "speculos",
                "app.elf"
            ]
        );
        assert_eq!(tty.args[1], "-it");
    }

    #[test]
    fn killing_a_run_passes_the_id_as_an_argument_not_inside_the_script() {
        let run_id = RunId::new("run-42").unwrap();
        let command = kill_run("app-x-container-stax", &run_id);

        assert_eq!(
            command.args[..6],
            ["exec", "--user", "0", "app-x-container-stax", "bash", "-c"]
        );
        assert!(!command.args[6].contains("run-42"));
        assert_eq!(command.args[7..], ["_", "run-42"]);
    }

    #[test]
    fn looking_at_docker_gives_up_quickly_and_the_registry_a_little_later() {
        assert_eq!(info().timeout, Some(Duration::from_secs(10)));
        assert_eq!(container_status("c").timeout, Some(Duration::from_secs(10)));
        assert_eq!(
            remote_manifest("img").timeout,
            Some(Duration::from_secs(20))
        );
        assert_eq!(pull("img").timeout, None);
    }

    #[test]
    fn reads_the_local_and_remote_digests() {
        assert_eq!(
            parse_local_digest("ghcr.io/x/y@sha256:abc\n"),
            Some("sha256:abc")
        );
        assert_eq!(parse_local_digest("no digest"), None);
        assert_eq!(parse_local_digest("repo@"), None);

        let json = r#"{"name":"x","manifest":{"digest":"sha256:abc","size":1}}"#;
        assert_eq!(parse_remote_digest(json), Some("sha256:abc".to_owned()));
        assert_eq!(parse_remote_digest("{}"), None);
        assert_eq!(parse_remote_digest("not json"), None);
    }

    #[test]
    fn the_status_and_digest_commands_are_plain_argument_lists() {
        assert_eq!(
            container_status("c").args,
            ["inspect", "-f", "{{.State.Status}} {{.Image}}", "c"]
        );
        assert_eq!(
            image_id("img").args,
            ["image", "inspect", "-f", "{{.Id}}", "img"]
        );
        assert_eq!(
            local_repo_digest("img").args,
            ["inspect", "--format={{index .RepoDigests 0}}", "img"]
        );
        assert_eq!(
            remote_manifest("img").args,
            [
                "buildx",
                "imagetools",
                "inspect",
                "img",
                "--format",
                "{{json .}}"
            ]
        );
    }
}
