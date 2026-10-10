use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;

use ledger_core::Command;

use crate::log::{Log, Stream};
use crate::runner::{CommandRunner, Output};

/// The id of the local image, until a pull brings another one.
pub const FIRST_IMAGE: &str = "sha256:image1";

/// A tiny in-memory Docker. It answers the commands of the container lifecycle, changes its
/// containers accordingly, records every command it was given, and logs like a real runner.
pub struct FakeDocker {
    state: Mutex<State>,
}

struct State {
    up: bool,
    /// The local image, when it is present.
    image_id: Option<String>,
    /// The image a pull brings.
    registry_image_id: String,
    /// Container name to Docker status and image id.
    containers: BTreeMap<String, (&'static str, String)>,
    calls: Vec<String>,
    /// Commands containing the text fail, this many more times.
    failures: Vec<(String, u32)>,
    repo_digest: Option<String>,
    remote_manifest: Option<String>,
}

impl FakeDocker {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                up: true,
                image_id: Some(FIRST_IMAGE.to_owned()),
                registry_image_id: FIRST_IMAGE.to_owned(),
                containers: BTreeMap::new(),
                calls: Vec::new(),
                failures: Vec::new(),
                repo_digest: None,
                remote_manifest: None,
            }),
        }
    }

    fn state_mut(&mut self) -> &mut State {
        self.state.get_mut().expect("the fake is not shared yet")
    }

    pub fn docker_down(mut self) -> Self {
        self.state_mut().up = false;
        self
    }

    pub fn without_image(mut self) -> Self {
        self.state_mut().image_id = None;
        self
    }

    /// A pull will bring this image.
    pub fn with_registry_image(mut self, image_id: &str) -> Self {
        self.state_mut().registry_image_id = image_id.to_owned();
        self
    }

    pub fn with_container(self, name: &str, status: &'static str) -> Self {
        self.with_container_image(name, status, FIRST_IMAGE)
    }

    pub fn with_container_image(
        mut self,
        name: &str,
        status: &'static str,
        image_id: &str,
    ) -> Self {
        self.state_mut()
            .containers
            .insert(name.to_owned(), (status, image_id.to_owned()));
        self
    }

    pub fn fail(mut self, command_text: &str, times: u32) -> Self {
        self.state_mut()
            .failures
            .push((command_text.to_owned(), times));
        self
    }

    pub fn with_digests(
        mut self,
        repo_digest: Option<&str>,
        remote_manifest: Option<&str>,
    ) -> Self {
        let state = self.state_mut();
        state.repo_digest = repo_digest.map(str::to_owned);
        state.remote_manifest = remote_manifest.map(str::to_owned);
        self
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn container_status(&self, name: &str) -> Option<&'static str> {
        self.state
            .lock()
            .unwrap()
            .containers
            .get(name)
            .map(|(status, _)| *status)
    }

    pub fn container_image(&self, name: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .containers
            .get(name)
            .map(|(_, image)| image.clone())
    }

    fn handle(&self, command: &Command) -> Output {
        let mut state = self.state.lock().unwrap();
        let line = command.shell_line();
        state.calls.push(line.clone());

        if command.program == "docker" && !state.up {
            return failure("Cannot connect to the Docker daemon");
        }
        if let Some((_, left)) = state
            .failures
            .iter_mut()
            .find(|(text, left)| *left > 0 && line.contains(text.as_str()))
        {
            *left -= 1;
            return failure("injected failure");
        }

        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        match (command.program.as_str(), args.as_slice()) {
            ("xhost", _) | ("docker", ["info"] | ["exec", ..]) => success(""),
            ("docker", ["inspect", "-f", _, name]) => match state.containers.get(*name) {
                Some((status, image)) => success(&format!("{status} {image}\n")),
                None => failure("No such object"),
            },
            ("docker", ["image", "inspect", "-f", _, _]) => match &state.image_id {
                Some(id) => success(&format!("{id}\n")),
                None => failure("No such image"),
            },
            ("docker", ["inspect", format, _]) if format.starts_with("--format=") => {
                match &state.repo_digest {
                    Some(digest) => success(digest),
                    None => failure("no digest"),
                }
            }
            ("docker", ["buildx", "imagetools", "inspect", ..]) => match &state.remote_manifest {
                Some(manifest) => success(manifest),
                None => failure("registry unreachable"),
            },
            ("docker", ["pull", _]) => {
                state.image_id = Some(state.registry_image_id.clone());
                success("")
            }
            ("docker", ["start", name]) => set_status(&mut state, name, "running"),
            ("docker", ["stop", name]) => set_status(&mut state, name, "exited"),
            ("docker", ["rm", name]) => {
                if state.containers.remove(*name).is_some() {
                    success("")
                } else {
                    failure("No such container")
                }
            }
            ("docker", ["run", rest @ ..]) => {
                let name = rest
                    .iter()
                    .position(|arg| *arg == "--name")
                    .and_then(|at| rest.get(at + 1));
                match (name, state.image_id.clone()) {
                    (Some(name), Some(image)) => {
                        state
                            .containers
                            .insert((*name).to_owned(), ("running", image));
                        success("0123abcd\n")
                    }
                    (Some(_), None) => failure("Unable to find image locally"),
                    (None, _) => failure("no --name given"),
                }
            }
            _ => panic!("the fake Docker does not know this command: {line}"),
        }
    }
}

impl CommandRunner for FakeDocker {
    fn run(
        &self,
        command: &Command,
        log: Option<&Log>,
    ) -> impl Future<Output = io::Result<Output>> + Send {
        let output = self.handle(command);
        if let Some(log) = log {
            log.started(command);
            output
                .stdout
                .lines()
                .for_each(|line| log.line(Stream::Stdout, line));
            output
                .stderr
                .lines()
                .for_each(|line| log.line(Stream::Stderr, line));
            log.finished(output.success);
        }
        async move { Ok(output) }
    }
}

fn set_status(state: &mut State, name: &str, status: &'static str) -> Output {
    match state.containers.get_mut(name) {
        Some((current, _)) => {
            *current = status;
            success("")
        }
        None => failure("No such container"),
    }
}

fn success(stdout: &str) -> Output {
    Output {
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

fn failure(stderr: &str) -> Output {
    Output {
        success: false,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}
