use std::sync::Mutex;
use std::time::Duration;

use ledger_core::Command;
use ledger_core::Device;
use ledger_core::actions::ContainerAction;
use ledger_core::c_app::{self, MakefileInfo};
use ledger_core::container::{
    self, ContainerState, Inspected, Observation, ObservedContainer, Operation, Plan, Retries,
    Retry, Settings, Status,
};
use ledger_core::docker::{self, ContainerUser, CreateContainer, RunId, RunMake};
use ledger_core::platform::Platform;
use ledger_core::task::{self, InContainer, Job, Step};
use ledger_core::update::{self, CheckPolicy, Freshness, LastCheck};

use crate::cancel::CancelToken;
use crate::log::{Log, LogSender, Source};
use crate::runner::{CommandRunner, Output, new_run_id};
use crate::runs::ActiveRuns;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub image: String,
    /// Name of the app folder, which every container name starts with.
    pub app_folder: String,
    /// Host path of the app folder, mounted into the containers.
    pub app_dir: String,
    /// Update the containers when the image has a newer version. Never replaces a container
    /// that has a run active.
    pub auto_update: bool,
    pub max_retries: u32,
    pub user: ContainerUser,
    /// The user's own `docker run` arguments, already split.
    pub extra_args: Vec<String>,
    pub platform: Platform,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing more to do. The status is `Stopped` only when there is no container to manage.
    Settled(Status),
    DockerUnavailable,
    /// The containers did not become ready within the allowed retries.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    Succeeded,
    /// The run was cancelled, and what it had started in the container was stopped.
    Cancelled,
    /// A step failed, counting from 0, and the job stopped there.
    Failed {
        step: usize,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult {
    pub device: Device,
    pub outcome: JobOutcome,
}

/// Keeps the Docker containers of one app, one per device, ready to run commands in.
///
/// With a log attached, the output of what it does is sent there, one stream per container
/// (and one for the shared image). Looking at the state of Docker is never logged.
pub struct ContainerManager<R> {
    runner: R,
    config: Config,
    logs: Option<LogSender>,
    runs: ActiveRuns,
    last_check: Mutex<Option<LastCheck>>,
}

impl<R: CommandRunner> ContainerManager<R> {
    pub fn new(runner: R, config: Config) -> Self {
        Self {
            runner,
            config,
            logs: None,
            runs: ActiveRuns::default(),
            last_check: Mutex::new(None),
        }
    }

    pub fn with_logs(mut self, logs: LogSender) -> Self {
        self.logs = Some(logs);
        self
    }

    /// Starts from an earlier answer of the registry, for example one saved before a restart.
    pub fn with_last_check(self, last_check: LastCheck) -> Self {
        *self.last_check.lock().unwrap() = Some(last_check);
        self
    }

    /// The runs that are active in the containers.
    pub fn runs(&self) -> &ActiveRuns {
        &self.runs
    }

    pub fn last_check(&self) -> Option<LastCheck> {
        *self.last_check.lock().unwrap()
    }

    fn record(&self, at: u64, freshness: Freshness) {
        *self.last_check.lock().unwrap() = Some(LastCheck { at, freshness });
    }

    fn freshness(&self) -> Freshness {
        self.last_check()
            .map_or(Freshness::Unknown, |check| check.freshness)
    }

    fn container_name(&self, device: Device) -> String {
        container::container_name(&self.config.app_folder, device)
    }

    fn log(&self, source: Source) -> Option<Log> {
        self.logs
            .as_ref()
            .map(|sender| Log::new(source, sender.clone()))
    }

    fn note(&self, source: Source, text: &str) {
        if let Some(log) = self.log(source) {
            log.note(text);
        }
    }

    /// Runs a command without logging it.
    async fn run(&self, command: &Command) -> Option<Output> {
        self.runner.run(command, None).await.ok()
    }

    async fn run_in(&self, source: Source, command: &Command) -> Option<Output> {
        let log = self.log(source);
        self.runner.run(command, log.as_ref()).await.ok()
    }

    async fn run_checked(&self, source: Source, command: &Command) -> Result<Output, String> {
        let log = self.log(source);
        self.check(command, log.as_ref()).await
    }

    async fn check(&self, command: &Command, log: Option<&Log>) -> Result<Output, String> {
        let output = self
            .runner
            .run(command, log)
            .await
            .map_err(|error| format!("{command}: {error}"))?;
        if output.success {
            Ok(output)
        } else {
            Err(format!("{command}: {}", output.stderr.trim()))
        }
    }

    async fn succeeds(&self, command: &Command) -> bool {
        self.run(command).await.is_some_and(|output| output.success)
    }

    /// What Docker reports about the containers of `devices`, and what is known of the registry.
    pub async fn observe(&self, devices: &[Device]) -> Observation {
        if !self.succeeds(&docker::info()).await {
            return Observation {
                docker_available: false,
                image_id: None,
                freshness: Freshness::Unknown,
                containers: Vec::new(),
            };
        }

        let local_image = self
            .run(&docker::image_id(&self.config.image))
            .await
            .filter(|output| output.success)
            .map(|output| output.stdout.trim().to_owned())
            .filter(|id| !id.is_empty());

        let pip_cache = docker::pip_cache_volume(self.config.user);
        let mut containers = Vec::with_capacity(devices.len());
        for &device in devices {
            let inspected = self
                .run(&docker::container_status(&self.container_name(device)))
                .await;
            let inspected = match inspected {
                Some(output) if output.success => container::parse_inspect(&output.stdout),
                _ => Inspected {
                    state: ContainerState::Missing,
                    image_id: None,
                    pip_cache: None,
                },
            };
            containers.push(ObservedContainer {
                device,
                state: inspected.state,
                image_id: inspected.image_id,
                pip_cache_ready: inspected.state == ContainerState::Missing
                    || inspected.pip_cache.as_deref() == Some(pip_cache.as_str()),
                busy: self.runs.is_busy(device),
            });
        }
        Observation {
            docker_available: true,
            image_id: local_image,
            freshness: self.freshness(),
            containers,
        }
    }

    pub async fn status(&self, devices: &[Device]) -> Status {
        self.observe(devices).await.status()
    }

    /// Asks the registry whether the image has a newer version, unless it was asked recently.
    /// An unreachable registry gives `Unknown`, which never triggers an update.
    pub async fn check_for_update(&self, now: u64, policy: &CheckPolicy) -> Freshness {
        let last = self.last_check();
        if !update::is_check_due(now, last, policy) {
            return last.map_or(Freshness::Unknown, |check| check.freshness);
        }
        let freshness = match self.image_outdated().await {
            Some(true) => Freshness::Outdated,
            Some(false) => Freshness::UpToDate,
            None => Freshness::Unknown,
        };
        self.record(now, freshness);
        freshness
    }

    /// Makes the containers of `devices` ready, retrying a limited number of times. With
    /// `auto_update`, also replaces the containers that run an old image, as long as no run is
    /// active in them. `now` is the unix time in seconds.
    pub async fn ensure_ready(&self, devices: &[Device], now: u64) -> Outcome {
        let settings = Settings {
            auto_update: self.config.auto_update,
        };
        let mut retries = Retries::new(self.config.max_retries);
        loop {
            let observation = self.observe(devices).await;
            let failure = match container::plan(&observation, &settings) {
                Plan::DockerUnavailable => return Outcome::DockerUnavailable,
                Plan::Nothing => return Outcome::Settled(observation.status()),
                Plan::Steps {
                    operations,
                    deferred,
                } => {
                    for &device in &deferred {
                        self.note(
                            Source::Container(device),
                            "update postponed: a run is active in this container",
                        );
                    }
                    self.execute(&operations, &observation, now).await.err()
                }
            };

            let status = self.status(devices).await;
            match retries.attempt_ended(status != Status::Stopped) {
                Retry::Done => return Outcome::Settled(status),
                Retry::Again => {}
                Retry::GiveUp => {
                    return Outcome::Failed(
                        failure.unwrap_or_else(|| "the containers are not running".to_owned()),
                    );
                }
            }
        }
    }

    async fn execute(
        &self,
        operations: &[Operation],
        observation: &Observation,
        now: u64,
    ) -> Result<(), String> {
        for operation in operations {
            match *operation {
                Operation::Pull => {
                    let pull = docker::pull(&self.config.image);
                    self.run_checked(Source::Image, &pull).await?;
                    self.record(now, Freshness::UpToDate);
                }
                Operation::Start(device) => {
                    let start = docker::start(&self.container_name(device));
                    self.run_checked(Source::Container(device), &start).await?;
                }
                Operation::Create(device) => {
                    self.create(device, observation.state_of(device)).await?;
                }
            }
        }
        Ok(())
    }

    /// Does what the user asked for the containers of `devices`, one container after the other.
    /// A container with an active run is left alone.
    pub async fn run_container_action(
        &self,
        action: ContainerAction,
        devices: &[Device],
        now: u64,
    ) -> Result<(), String> {
        let observation = self.observe(devices).await;
        if !observation.docker_available {
            return Err("Docker is not available".to_owned());
        }
        let mut operations = Vec::new();
        match action {
            ContainerAction::Update => operations.push(Operation::Pull),
            ContainerAction::Create if !observation.image_present() => {
                return Err(
                    "the image is not there yet: update the containers to pull it".to_owned(),
                );
            }
            ContainerAction::Create => {}
        }
        for &device in devices {
            if self.runs.is_busy(device) {
                self.note(
                    Source::Container(device),
                    "not replaced: a run is active in this container",
                );
            } else {
                operations.push(Operation::Create(device));
            }
        }
        self.execute(&operations, &observation, now).await
    }

    async fn create(&self, device: Device, existing: ContainerState) -> Result<(), String> {
        let source = Source::Container(device);
        let name = self.container_name(device);
        if let Some(allow_x11) = docker::allow_x11(self.config.platform) {
            self.run_in(source.clone(), &allow_x11).await;
        }
        if existing != ContainerState::Missing {
            self.run_in(source.clone(), &docker::stop(&name)).await;
            self.run_checked(source.clone(), &docker::remove(&name))
                .await?;
        }
        // The containers of the user share one cache of what pip downloads and builds. Docker makes
        // its volume as root's, so the user is given it first: pip does not write to anything else.
        let pip_cache = docker::pip_cache_volume(self.config.user);
        self.run_in(source.clone(), &docker::create_volume(&pip_cache))
            .await;
        if let Some(own) = docker::own_pip_cache(&self.config.image, &pip_cache, self.config.user) {
            self.run_in(source.clone(), &own).await;
        }
        let create = docker::create_container(&CreateContainer {
            name: &name,
            image: &self.config.image,
            app_dir: &self.config.app_dir,
            user: self.config.user,
            pip_cache: &pip_cache,
            extra_args: &self.config.extra_args,
            platform: self.config.platform,
            display: self.config.display.as_deref(),
        });
        self.run_checked(source, &create).await?;
        Ok(())
    }

    pub async fn stop(&self, device: Device) -> bool {
        let stop = docker::stop(&self.container_name(device));
        self.run_in(Source::Container(device), &stop)
            .await
            .is_some_and(|output| output.success)
    }

    pub async fn remove(&self, device: Device) -> bool {
        let remove = docker::remove(&self.container_name(device));
        self.run_in(Source::Container(device), &remove)
            .await
            .is_some_and(|output| output.success)
    }

    /// Stops every process of a run in the container of `device`, wherever it was started from.
    pub async fn kill_run(&self, device: Device, run_id: &RunId) -> bool {
        let command = docker::kill_run(&self.container_name(device), run_id);
        self.succeeds(&command).await
    }

    /// Asks every run that is active in the container of `device` to stop. Returns how many.
    pub async fn cancel(&self, device: Device) -> usize {
        let ids = self.runs.ids(device);
        for run_id in &ids {
            self.kill_run(device, run_id).await;
        }
        ids.len()
    }

    /// Runs the jobs of a plan side by side, each in the container of its device. The processes
    /// of a job are tagged and stopped when it ends, so nothing is left running in the container.
    pub async fn run_plan(&self, plan: &task::Plan) -> Vec<JobResult> {
        self.run_plan_until(plan, &CancelToken::new()).await
    }

    /// Like [`ContainerManager::run_plan`], and every job stops when `cancel` is cancelled: what
    /// runs on the host is killed, and so is what it started in the container.
    pub async fn run_plan_until(&self, plan: &task::Plan, cancel: &CancelToken) -> Vec<JobResult> {
        futures_util::future::join_all(plan.jobs.iter().map(|job| self.run_job(job, cancel))).await
    }

    async fn run_job(&self, job: &Job, cancel: &CancelToken) -> JobResult {
        let run_id = new_run_id();
        let in_container = job.uses_container();
        let _active = in_container.then(|| self.runs.begin(job.device, run_id.clone()));
        // Dropping the steps kills their clients. The cleanup below runs either way, and it is
        // what stops the processes in the container.
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => JobOutcome::Cancelled,
            outcome = self.run_steps(job, &run_id) => outcome,
        };
        if in_container {
            self.kill_run(job.device, &run_id).await;
        }
        JobResult {
            device: job.device,
            outcome,
        }
    }

    async fn run_steps(&self, job: &Job, run_id: &RunId) -> JobOutcome {
        let container = self.container_name(job.device);
        let source = Source::Container(job.device);
        for (index, step) in job.steps.iter().enumerate() {
            let command = match step {
                Step::InContainer(step) => step.command(&container, self.config.user, run_id),
                Step::OnHost(host) => host.command.clone(),
            };
            let Err(reason) = self.run_checked(source.clone(), &command).await else {
                continue;
            };
            if !step.continue_on_error() {
                return JobOutcome::Failed {
                    step: index,
                    reason,
                };
            }
            self.note(
                source.clone(),
                &format!("step {} failed, going on", index + 1),
            );
        }
        JobOutcome::Succeeded
    }

    /// Asks the Makefile of the C app in `build_directory` for its name and variants. The image
    /// must be local already, and the answer is not logged.
    pub async fn makefile_info(&self, build_directory: &str) -> Result<MakefileInfo, String> {
        let listing = self.make(build_directory, c_app::LIST_INFO_TARGET).await?;
        let name = c_app::parse_app_name(&listing)
            .ok_or_else(|| "make listinfo printed no APPNAME".to_owned())?;
        // Older SDKs have no `listvariants`: the app has none then.
        let variants = self
            .make(build_directory, c_app::LIST_VARIANTS_TARGET)
            .await
            .ok()
            .and_then(|listing| c_app::parse_variants(&listing));
        Ok(MakefileInfo { name, variants })
    }

    /// The service to open a terminal in: the first one of the compose file, which is at `file`
    /// on the host.
    pub async fn compose_service(&self, file: &str) -> Result<String, String> {
        let output = self.check(&docker::compose_services(file), None).await?;
        docker::parse_first_service(&output.stdout)
            .map(str::to_owned)
            .ok_or_else(|| format!("{file} lists no service"))
    }

    async fn make(&self, build_directory: &str, target: &str) -> Result<String, String> {
        let command = docker::run_make(&RunMake {
            image: &self.config.image,
            app_dir: &self.config.app_dir,
            build_directory,
            user: self.config.user,
            target,
        });
        let output = self.check(&command, None).await?;
        Ok(output.stdout)
    }

    /// Runs a step in the container of `device` and returns what it printed. It is a question, so
    /// it is not logged, and the processes it started are stopped when it ends.
    pub async fn capture(
        &self,
        device: Device,
        step: &InContainer,
        timeout: Duration,
    ) -> Result<String, String> {
        let run_id = new_run_id();
        let _active = self.runs.begin(device, run_id.clone());
        let command = step
            .command(&self.container_name(device), self.config.user, &run_id)
            .timeout(timeout);
        let result = self.runner.run(&command, None).await;
        self.kill_run(device, &run_id).await;
        let output = result.map_err(|error| format!("{command}: {error}"))?;
        if output.success {
            Ok(output.stdout)
        } else {
            Err(format!("{command}: {}", last_lines(&output, 12)))
        }
    }

    /// Whether the registry has a newer image than the local one. `None` when either digest is
    /// unknown, for example when offline.
    pub async fn image_outdated(&self) -> Option<bool> {
        let local = self
            .run(&docker::local_repo_digest(&self.config.image))
            .await
            .filter(|output| output.success)?;
        let local = docker::parse_local_digest(&local.stdout)?;
        let remote = self
            .run(&docker::remote_manifest(&self.config.image))
            .await
            .filter(|output| output.success)?;
        let remote = docker::parse_remote_digest(&remote.stdout)?;
        Some(local != remote)
    }
}

/// The end of what a command printed, where its error usually is.
fn last_lines(output: &Output, count: usize) -> String {
    let text = format!("{}\n{}", output.stdout, output.stderr);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::Language;
    use ledger_core::Target;
    use ledger_core::app::App;
    use ledger_core::task::{Options, Task, TestMode, TestRun};

    use crate::fake::FakeDocker;
    use crate::log::{self, LogEvent, LogReceiver, Stream};

    const STAX: &str = "app-x-container-stax";
    const FLEX: &str = "app-x-container-flex";
    const NOW: u64 = 1_000;
    const MUTATIONS: [&str; 5] = [
        "docker start",
        "docker stop",
        "docker rm",
        "docker run",
        "docker pull",
    ];

    fn config() -> Config {
        Config {
            image: "img:latest".to_owned(),
            app_folder: "app-x".to_owned(),
            app_dir: "/home/u/app-x".to_owned(),
            auto_update: false,
            max_retries: 2,
            user: ContainerUser::Host {
                uid: 1000,
                gid: 1000,
            },
            extra_args: Vec::new(),
            platform: Platform::Linux,
            display: Some(":0".to_owned()),
        }
    }

    fn auto() -> Config {
        Config {
            auto_update: true,
            ..config()
        }
    }

    fn remote(digest: &str) -> String {
        format!(r#"{{"manifest":{{"digest":"{digest}"}}}}"#)
    }

    /// A registry with a newer image than the local one.
    fn newer_registry(docker: FakeDocker) -> FakeDocker {
        docker
            .with_digests(Some("repo@sha256:aaa"), Some(&remote("sha256:bbb")))
            .with_registry_image("sha256:image2")
    }

    fn count(docker: &FakeDocker, prefix: &str) -> usize {
        docker
            .calls()
            .iter()
            .filter(|call| call.starts_with(prefix))
            .count()
    }

    fn position(docker: &FakeDocker, prefix: &str) -> usize {
        docker
            .calls()
            .iter()
            .position(|call| call.starts_with(prefix))
            .unwrap_or_else(|| panic!("no call starts with {prefix}"))
    }

    fn mutations(docker: &FakeDocker) -> usize {
        MUTATIONS.iter().map(|prefix| count(docker, prefix)).sum()
    }

    fn drain(receiver: &mut LogReceiver) -> Vec<LogEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn started(events: &[LogEvent], wanted: &Source) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                LogEvent::Started { source, command } if source == wanted => Some(command.clone()),
                _ => None,
            })
            .collect()
    }

    fn finished(events: &[LogEvent], wanted: &Source) -> Vec<bool> {
        events
            .iter()
            .filter_map(|event| match event {
                LogEvent::Finished { source, success } if source == wanted => Some(*success),
                _ => None,
            })
            .collect()
    }

    fn notes(events: &[LogEvent], wanted: &Source) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                LogEvent::Note { source, text } if source == wanted => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn without_docker_nothing_is_attempted() {
        let docker = FakeDocker::new().docker_down();

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(outcome, Outcome::DockerUnavailable);
        assert_eq!(docker.calls(), ["docker info"]);
    }

    #[tokio::test]
    async fn running_containers_are_left_alone() {
        let docker = FakeDocker::new()
            .with_container(STAX, "running")
            .with_container(FLEX, "running");

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn a_restarting_container_counts_as_syncing_and_is_not_touched() {
        let docker = FakeDocker::new().with_container(STAX, "restarting");

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Syncing));
        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn a_stopped_container_is_started_without_recreating_anything() {
        let docker = FakeDocker::new()
            .with_container(STAX, "exited")
            .with_container(FLEX, "running");

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, &format!("docker start {STAX}")), 1);
        assert_eq!(mutations(&docker), 1);
        assert_eq!(docker.container_status(STAX), Some("running"));
    }

    #[tokio::test]
    async fn a_missing_container_is_created_from_the_local_image_without_pulling() {
        let docker = FakeDocker::new();

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, "docker pull"), 0);
        assert_eq!(count(&docker, "xhost"), 1);
        assert_eq!(count(&docker, "docker run --init"), 1);
        assert_eq!(docker.container_status(STAX), Some("running"));
    }

    #[tokio::test]
    async fn the_image_is_pulled_before_creating_when_it_is_missing_locally() {
        let docker = FakeDocker::new().without_image();

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert!(position(&docker, "docker pull") < position(&docker, "docker run"));
    }

    #[tokio::test]
    async fn a_failing_start_is_retried_then_reported() {
        let docker = FakeDocker::new()
            .with_container(STAX, "exited")
            .fail("docker start", 99);

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert!(matches!(&outcome, Outcome::Failed(reason) if reason.contains("docker start")));
        assert_eq!(count(&docker, "docker start"), 3);
    }

    #[tokio::test]
    async fn a_start_that_fails_once_is_retried_successfully() {
        let docker = FakeDocker::new()
            .with_container(STAX, "exited")
            .fail("docker start", 1);

        let outcome = ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, "docker start"), 2);
    }

    #[tokio::test]
    async fn the_image_is_outdated_when_the_digests_differ() {
        let outdated = |local: &str, remote_digest: &str| {
            let docker = FakeDocker::new().with_digests(Some(local), Some(&remote(remote_digest)));
            async move {
                ContainerManager::new(docker, config())
                    .image_outdated()
                    .await
            }
        };

        assert_eq!(outdated("repo@sha256:aaa", "sha256:bbb").await, Some(true));
        assert_eq!(outdated("repo@sha256:aaa", "sha256:aaa").await, Some(false));
    }

    #[tokio::test]
    async fn the_image_freshness_is_unknown_when_the_registry_is_unreachable() {
        let docker = FakeDocker::new().with_digests(Some("repo@sha256:aaa"), None);

        let outdated = ContainerManager::new(&docker, config())
            .image_outdated()
            .await;

        assert_eq!(outdated, None);
    }

    #[tokio::test]
    async fn killing_a_run_runs_the_cleanup_inside_the_device_container() {
        let docker = FakeDocker::new();
        let run_id = RunId::new("run-1").unwrap();

        let killed = ContainerManager::new(&docker, config())
            .kill_run(Device::Stax, &run_id)
            .await;

        assert!(killed);
        let call = docker.calls().pop().unwrap();
        assert!(call.starts_with(&format!("docker exec --user 0 {STAX} bash -c")));
        assert!(call.ends_with(" _ run-1"));
    }

    #[tokio::test]
    async fn cancelling_stops_every_active_run_of_the_device() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let first = manager
            .runs()
            .begin(Device::Stax, RunId::new("run-1").unwrap());
        let _second = manager
            .runs()
            .begin(Device::Stax, RunId::new("run-2").unwrap());
        let _other_device = manager
            .runs()
            .begin(Device::Flex, RunId::new("run-3").unwrap());
        assert_eq!(first.run_id().as_str(), "run-1");

        let cancelled = manager.cancel(Device::Stax).await;

        assert_eq!(cancelled, 2);
        assert_eq!(
            count(&docker, "docker exec --user 0 app-x-container-stax"),
            2
        );
        assert_eq!(
            count(&docker, "docker exec --user 0 app-x-container-flex"),
            0
        );
    }

    #[tokio::test]
    async fn each_container_has_its_own_log_stream() {
        let docker = FakeDocker::new();
        let (sender, mut receiver) = log::channel();

        let outcome = ContainerManager::new(&docker, config())
            .with_logs(sender)
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        let events = drain(&mut receiver);
        for (device, own, other) in [(Device::Stax, STAX, FLEX), (Device::Flex, FLEX, STAX)] {
            let source = Source::Container(device);
            let commands = started(&events, &source);
            assert_eq!(commands.len(), 4, "{commands:?}");
            assert_eq!(commands[0], "xhost +");
            assert!(commands[1].starts_with("docker volume create"));
            assert!(commands[2].contains("chown"));
            assert!(commands[3].starts_with("docker run --init"));
            assert!(commands[3].contains(own) && !commands[3].contains(other));
            assert_eq!(finished(&events, &source), [true; 4]);
        }
        assert!(started(&events, &Source::Image).is_empty());
    }

    #[tokio::test]
    async fn looking_at_docker_is_never_logged() {
        let docker = FakeDocker::new()
            .with_container(STAX, "running")
            .with_container(FLEX, "running");
        let (sender, mut receiver) = log::channel();

        ContainerManager::new(&docker, config())
            .with_logs(sender)
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert!(drain(&mut receiver).is_empty());
    }

    #[tokio::test]
    async fn pulling_the_shared_image_has_its_own_stream() {
        let docker = FakeDocker::new().without_image();
        let (sender, mut receiver) = log::channel();

        ContainerManager::new(&docker, config())
            .with_logs(sender)
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        let events = drain(&mut receiver);
        assert_eq!(started(&events, &Source::Image), ["docker pull img:latest"]);
        assert_eq!(finished(&events, &Source::Image), [true]);
    }

    #[tokio::test]
    async fn a_failing_step_shows_its_error_in_the_stream_of_its_container() {
        let docker = FakeDocker::new()
            .with_container(STAX, "exited")
            .fail("docker start", 99);
        let (sender, mut receiver) = log::channel();
        let config = Config {
            max_retries: 0,
            ..config()
        };

        ContainerManager::new(&docker, config)
            .with_logs(sender)
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        let events = drain(&mut receiver);
        let source = Source::Container(Device::Stax);
        assert!(events.iter().any(|event| matches!(
            event,
            LogEvent::Line { source: line_source, stream: Stream::Stderr, text }
                if *line_source == source && text == "injected failure"
        )));
        assert_eq!(finished(&events, &source), [false]);
    }

    #[tokio::test]
    async fn a_newer_image_is_pulled_once_and_idle_containers_are_replaced() {
        let docker = newer_registry(FakeDocker::new())
            .with_container(STAX, "running")
            .with_container(FLEX, "running");
        let manager = ContainerManager::new(&docker, auto());
        let both = [Device::Stax, Device::Flex];

        let freshness = manager.check_for_update(NOW, &CheckPolicy::default()).await;
        let outcome = manager.ensure_ready(&both, NOW).await;

        assert_eq!(freshness, Freshness::Outdated);
        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, "docker pull"), 1);
        assert!(position(&docker, "docker pull") < position(&docker, "docker stop"));
        assert_eq!(count(&docker, "docker run --init"), 2);
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image2")
        );
        assert_eq!(
            docker.container_image(FLEX).as_deref(),
            Some("sha256:image2")
        );
        assert_eq!(
            manager.last_check(),
            Some(LastCheck {
                at: NOW,
                freshness: Freshness::UpToDate
            })
        );

        let before = mutations(&docker);
        manager.ensure_ready(&both, NOW).await;
        assert_eq!(mutations(&docker), before, "nothing is left to update");
    }

    #[tokio::test]
    async fn a_newer_image_changes_nothing_without_auto_update() {
        let docker = newer_registry(FakeDocker::new()).with_container(STAX, "running");
        let manager = ContainerManager::new(&docker, config());

        manager.check_for_update(NOW, &CheckPolicy::default()).await;
        manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn an_unreachable_registry_never_triggers_an_update() {
        let docker = FakeDocker::new()
            .with_digests(Some("repo@sha256:aaa"), None)
            .with_container(STAX, "running");
        let manager = ContainerManager::new(&docker, auto());

        let freshness = manager.check_for_update(NOW, &CheckPolicy::default()).await;
        manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(freshness, Freshness::Unknown);
        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn a_container_with_a_run_active_is_not_replaced_but_the_image_is_pulled() {
        let docker = newer_registry(FakeDocker::new())
            .with_container(STAX, "running")
            .with_container(FLEX, "running");
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, auto()).with_logs(sender);
        let _run = manager
            .runs()
            .begin(Device::Stax, RunId::new("run-1").unwrap());

        manager.check_for_update(NOW, &CheckPolicy::default()).await;
        let outcome = manager
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, "docker pull"), 1);
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image1")
        );
        assert_eq!(
            docker.container_image(FLEX).as_deref(),
            Some("sha256:image2")
        );
        let events = drain(&mut receiver);
        assert_eq!(notes(&events, &Source::Container(Device::Stax)).len(), 1);
        assert!(notes(&events, &Source::Container(Device::Flex)).is_empty());
    }

    #[tokio::test]
    async fn the_deferred_container_is_replaced_once_its_run_is_over() {
        let docker = newer_registry(FakeDocker::new()).with_container(STAX, "running");
        let manager = ContainerManager::new(&docker, auto());
        let run = manager
            .runs()
            .begin(Device::Stax, RunId::new("run-1").unwrap());
        manager.check_for_update(NOW, &CheckPolicy::default()).await;
        manager.ensure_ready(&[Device::Stax], NOW).await;
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image1")
        );

        drop(run);
        manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image2")
        );
        assert_eq!(
            count(&docker, "docker pull"),
            1,
            "the image is not pulled twice"
        );
    }

    #[tokio::test]
    async fn a_failed_pull_leaves_the_containers_untouched_and_is_not_hammered() {
        let docker = newer_registry(FakeDocker::new())
            .with_container(STAX, "running")
            .fail("docker pull", 99);
        let manager = ContainerManager::new(&docker, auto());

        manager.check_for_update(NOW, &CheckPolicy::default()).await;
        let outcome = manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(outcome, Outcome::Settled(Status::Running));
        assert_eq!(count(&docker, "docker pull"), 1);
        assert_eq!(count(&docker, "docker stop"), 0);
        assert_eq!(count(&docker, "docker rm"), 0);
        assert_eq!(count(&docker, "docker run --init"), 0);
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image1")
        );
    }

    #[tokio::test]
    async fn a_container_made_from_an_older_local_image_is_replaced_without_pulling() {
        let docker = FakeDocker::new()
            .with_container_image(STAX, "running", "sha256:old")
            .with_container(FLEX, "running");

        ContainerManager::new(&docker, config())
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;
        assert_eq!(
            mutations(&docker),
            0,
            "without auto update it is left alone"
        );

        ContainerManager::new(&docker, auto())
            .ensure_ready(&[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(count(&docker, "docker pull"), 0);
        assert_eq!(count(&docker, "docker run --init"), 1);
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some(crate::fake::FIRST_IMAGE)
        );
    }

    #[tokio::test]
    async fn the_registry_is_not_asked_again_before_the_interval_is_over() {
        let docker = newer_registry(FakeDocker::new());
        let manager = ContainerManager::new(&docker, auto());
        let policy = CheckPolicy {
            interval: 100,
            retry_after_failure: 10,
        };

        manager.check_for_update(NOW, &policy).await;
        manager.check_for_update(NOW + 99, &policy).await;
        assert_eq!(count(&docker, "docker buildx"), 1);

        manager.check_for_update(NOW + 100, &policy).await;
        assert_eq!(count(&docker, "docker buildx"), 2);
    }

    #[tokio::test]
    async fn an_unreachable_registry_is_asked_again_sooner_but_not_at_once() {
        let docker = FakeDocker::new().with_digests(Some("repo@sha256:aaa"), None);
        let manager = ContainerManager::new(&docker, auto());
        let policy = CheckPolicy {
            interval: 100,
            retry_after_failure: 10,
        };

        manager.check_for_update(NOW, &policy).await;
        manager.check_for_update(NOW + 9, &policy).await;
        assert_eq!(count(&docker, "docker buildx"), 1);

        manager.check_for_update(NOW + 10, &policy).await;
        assert_eq!(count(&docker, "docker buildx"), 2);
    }

    fn plan_for(task: Task, devices: &[Device], options: &Options) -> task::Plan {
        let app = App {
            folder: "/work/app-x".into(),
            language: Language::C,
            build_directory: "./".to_owned(),
            devices: vec![Device::Stax, Device::Flex],
            rust: None,
            build_use_cases: Vec::new(),
            test_suites: Vec::new(),
            makefile: None,
            compose_file: None,
        };
        let target = Target::new(devices.iter().copied()).unwrap();
        task::plan_task(task, &app, &target, options).unwrap()
    }

    fn calls_in(docker: &FakeDocker, container: &str) -> Vec<String> {
        docker
            .calls()
            .into_iter()
            .filter(|call| call.contains(&format!(" {container} bash -c")))
            .collect()
    }

    #[tokio::test]
    async fn a_plan_runs_each_job_in_its_own_container_and_log_stream() {
        let docker = FakeDocker::new();
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);
        let plan = plan_for(
            Task::BuildApp,
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );

        let results = manager.run_plan(&plan).await;

        let succeeded = |device| JobResult {
            device,
            outcome: JobOutcome::Succeeded,
        };
        assert_eq!(results, [succeeded(Device::Stax), succeeded(Device::Flex)]);
        let events = drain(&mut receiver);
        for (device, name) in [(Device::Stax, STAX), (Device::Flex, FLEX)] {
            let source = Source::Container(device);
            assert_eq!(started(&events, &source), ["make -C ./ -j"]);
            assert_eq!(finished(&events, &source), [true]);

            let calls = calls_in(&docker, name);
            assert_eq!(calls.len(), 2, "the step, then the cleanup: {calls:?}");
            assert!(calls[0].starts_with("docker exec -i --user 1000:1000 -e "));
            assert!(calls[0].contains("LEDGER_RUN_ID="));
            assert!(calls[1].starts_with(&format!("docker exec --user 0 {name} bash -c")));
            assert!(!manager.runs().is_busy(device), "the run is over");
        }
    }

    #[tokio::test]
    async fn a_failing_step_stops_its_job_but_the_cleanup_still_runs() {
        let docker = FakeDocker::new().fail("clean_target", 99);
        let manager = ContainerManager::new(&docker, config());
        let plan = plan_for(Task::CleanAndBuild, &[Device::Stax], &Options::default());

        let results = manager.run_plan(&plan).await;

        assert!(matches!(
            &results[0].outcome,
            JobOutcome::Failed { step: 0, reason } if reason.contains("make -C ./ clean_target")
        ));
        let calls = calls_in(&docker, STAX);
        assert_eq!(
            calls.len(),
            2,
            "the failed step and the cleanup, not the build"
        );
        assert!(calls.iter().all(|call| !call.contains(" -j")));
    }

    #[tokio::test]
    async fn a_step_that_keeps_going_does_not_stop_the_job() {
        let docker = FakeDocker::new().fail("pip install extra", 99);
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);
        let options = Options {
            tests: Some(TestRun {
                directory: "tests/standalone".to_owned(),
                paths: Vec::new(),
            }),
            test_requirements: Some("pip install extra".to_owned()),
            ..Options::default()
        };
        let plan = plan_for(
            Task::RunTests(TestMode::Headless),
            &[Device::Stax],
            &options,
        );

        let results = manager.run_plan(&plan).await;

        assert_eq!(results[0].outcome, JobOutcome::Succeeded);
        assert!(
            calls_in(&docker, STAX)
                .iter()
                .any(|call| call.contains("pytest"))
        );
        let events = drain(&mut receiver);
        assert_eq!(
            notes(&events, &Source::Container(Device::Stax)),
            ["step 1 failed, going on"]
        );
    }

    #[tokio::test]
    async fn a_device_is_busy_for_the_duration_of_its_run() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let plan = plan_for(Task::BuildApp, &[Device::Stax], &Options::default());

        assert!(!manager.runs().is_busy(Device::Stax));
        manager.run_plan(&plan).await;

        assert!(!manager.runs().is_busy(Device::Stax));
    }

    #[tokio::test]
    async fn a_saved_answer_is_reused_after_a_restart() {
        let docker = newer_registry(FakeDocker::new());
        let saved = LastCheck {
            at: NOW,
            freshness: Freshness::Outdated,
        };
        let manager = ContainerManager::new(&docker, auto()).with_last_check(saved);

        let freshness = manager
            .check_for_update(NOW + 60, &CheckPolicy::default())
            .await;

        assert_eq!(freshness, Freshness::Outdated);
        assert_eq!(count(&docker, "docker buildx"), 0);
    }

    fn makefile_docker() -> FakeDocker {
        FakeDocker::new()
            .with_make_output(
                "listinfo",
                "make: Entering directory '/app'\nAPPNAME=Boilerplate\n",
            )
            .with_make_output("listvariants", "VARIANTS COIN BTC LTC\n")
    }

    #[tokio::test]
    async fn the_makefile_is_asked_for_the_name_and_the_variants() {
        let docker = makefile_docker();
        let manager = ContainerManager::new(&docker, config());

        let info = manager.makefile_info("./app").await.unwrap();

        assert_eq!(info.name, "Boilerplate");
        let variants = info.variants.unwrap();
        assert_eq!(variants.name, "COIN");
        assert_eq!(variants.values, ["BTC", "LTC"]);
        assert_eq!(
            docker.calls()[0],
            "docker run --rm --pull never --user 1000:1000 -v /home/u/app-x:/app -w /app \
             -e 'BOLOS_SDK=/opt/stax-secure-sdk' img:latest make -C ./app listinfo"
        );
    }

    #[tokio::test]
    async fn an_app_without_variants_still_has_its_name() {
        let docker = FakeDocker::new().with_make_output("listinfo", "APPNAME=Boilerplate\n");
        let manager = ContainerManager::new(&docker, config());

        let info = manager.makefile_info("./").await.unwrap();

        assert_eq!(info.name, "Boilerplate");
        assert_eq!(info.variants, None);
    }

    #[tokio::test]
    async fn a_makefile_that_prints_no_name_is_an_error() {
        let docker = FakeDocker::new().with_make_output("listinfo", "nothing useful\n");
        let manager = ContainerManager::new(&docker, config());

        let error = manager.makefile_info("./").await.unwrap_err();

        assert!(error.contains("APPNAME"), "{error}");
    }

    #[tokio::test]
    async fn a_missing_image_is_reported_and_never_pulled() {
        let docker = makefile_docker().without_image();
        let manager = ContainerManager::new(&docker, config());

        let error = manager.makefile_info("./").await.unwrap_err();

        assert!(error.contains("Unable to find image"), "{error}");
        assert_eq!(count(&docker, "docker pull"), 0);
    }

    #[tokio::test]
    async fn asking_the_makefile_is_not_logged() {
        let docker = makefile_docker();
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);

        manager.makefile_info("./").await.unwrap();

        assert!(drain(&mut receiver).is_empty());
    }

    fn host_plan(commands: &[&[&str]]) -> task::Plan {
        let steps = commands
            .iter()
            .map(|argv| Step::on_host(Command::new(argv[0]).args(argv[1..].iter().copied())));
        task::Plan {
            jobs: vec![Job {
                device: Device::Stax,
                steps: steps.collect(),
            }],
            warnings: Vec::new(),
        }
    }

    #[tokio::test]
    async fn host_steps_run_on_the_host_and_leave_the_container_alone() {
        let docker = FakeDocker::new();
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);
        let plan = host_plan(&[&["python3", "-m", "venv", "/v"], &["/v/bin/python", "-V"]]);

        let results = manager.run_plan(&plan).await;

        assert_eq!(results[0].outcome, JobOutcome::Succeeded);
        assert_eq!(docker.calls(), ["python3 -m venv /v", "/v/bin/python -V"]);
        let events = drain(&mut receiver);
        assert!(matches!(
            &events[0],
            LogEvent::Started { source: Source::Container(Device::Stax), command }
                if command == "python3 -m venv /v"
        ));
    }

    #[tokio::test]
    async fn a_host_step_that_fails_stops_the_job() {
        let docker = FakeDocker::new().fail("pip install", 1);
        let manager = ContainerManager::new(&docker, config());
        let plan = host_plan(&[
            &["python3", "-m", "venv", "/v"],
            &["/v/bin/python", "-m", "pip", "install", "ledgerblue"],
            &["/v/bin/python", "-m", "ledgerblue.runScript"],
        ]);

        let results = manager.run_plan(&plan).await;

        assert!(matches!(
            &results[0].outcome,
            JobOutcome::Failed { step: 1, reason } if reason.contains("pip install")
        ));
        assert_eq!(docker.calls().len(), 2, "the load was not attempted");
    }

    #[tokio::test]
    async fn a_host_step_that_keeps_going_lets_the_others_run_and_a_failing_check_ends_the_job() {
        use ledger_core::task::OnHost;

        // The first repository cannot be fetched. The second one still is, and the check that
        // lists what is missing comes last and fails the job.
        let docker = FakeDocker::new()
            .fail("fetch app-bitcoin-dev", 128)
            .fail("check missing", 1);
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);
        let keeping_going = |argv: &[&str]| {
            Step::OnHost(
                OnHost::new(Command::new(argv[0]).args(argv[1..].iter().copied())).keep_going(),
            )
        };
        let plan = task::Plan {
            jobs: vec![Job {
                device: Device::Stax,
                steps: vec![
                    keeping_going(&["git", "fetch", "app-bitcoin-dev"]),
                    keeping_going(&["git", "fetch", "app-tezos-dev"]),
                    Step::on_host(Command::new("check").arg("missing")),
                ],
            }],
            warnings: Vec::new(),
        };

        let results = manager.run_plan(&plan).await;

        assert!(matches!(
            &results[0].outcome,
            JobOutcome::Failed { step: 2, reason } if reason.contains("check missing")
        ));
        assert_eq!(
            docker.calls(),
            [
                "git fetch app-bitcoin-dev",
                "git fetch app-tezos-dev",
                "check missing"
            ],
            "the repository after the one that failed was still fetched"
        );
        let events = drain(&mut receiver);
        assert_eq!(
            notes(&events, &Source::Container(Device::Stax)),
            ["step 1 failed, going on"]
        );
    }

    #[tokio::test]
    async fn a_host_step_that_keeps_going_and_fails_does_not_fail_a_job_that_ends_well() {
        use ledger_core::task::OnHost;

        let docker = FakeDocker::new().fail("fetch app-bitcoin-dev", 128);
        let manager = ContainerManager::new(&docker, config());
        let plan = task::Plan {
            jobs: vec![Job {
                device: Device::Stax,
                steps: vec![
                    Step::OnHost(
                        OnHost::new(Command::new("git").args(["fetch", "app-bitcoin-dev"]))
                            .keep_going(),
                    ),
                    Step::on_host(Command::new("check").arg("everything")),
                ],
            }],
            warnings: Vec::new(),
        };

        let results = manager.run_plan(&plan).await;

        assert_eq!(results[0].outcome, JobOutcome::Succeeded);
        assert_eq!(docker.calls().len(), 2);
    }

    #[tokio::test]
    async fn a_device_job_on_the_host_does_not_make_the_container_busy() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let plan = host_plan(&[&["python3", "-V"]]);

        manager.run_plan(&plan).await;

        assert_eq!(count(&docker, "docker"), 0, "no cleanup in the container");
    }

    #[tokio::test]
    async fn the_compose_service_is_the_first_one_the_file_lists() {
        let docker = FakeDocker::new().with_compose_services("builder\ndb\n");
        let manager = ContainerManager::new(&docker, config());

        let service = manager
            .compose_service("/home/u/app-x/docker-compose.yml")
            .await;

        assert_eq!(service, Ok("builder".to_owned()));
        assert_eq!(
            docker.calls(),
            ["docker compose -f /home/u/app-x/docker-compose.yml config --services"]
        );
    }

    #[tokio::test]
    async fn a_missing_compose_file_or_one_without_services_is_an_error() {
        let missing = FakeDocker::new();
        let empty = FakeDocker::new().with_compose_services("\n");

        let no_file = ContainerManager::new(&missing, config())
            .compose_service("/w/docker-compose.yml")
            .await;
        let no_service = ContainerManager::new(&empty, config())
            .compose_service("/w/docker-compose.yml")
            .await;

        assert!(no_file.unwrap_err().contains("configuration file"));
        assert!(no_service.unwrap_err().contains("lists no service"));
    }

    #[tokio::test]
    async fn updating_pulls_the_image_and_creates_the_containers_again() {
        let docker = newer_registry(FakeDocker::new())
            .with_container(STAX, "running")
            .with_container(FLEX, "exited");
        let manager = ContainerManager::new(&docker, config());

        let result = manager
            .run_container_action(ContainerAction::Update, &[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(result, Ok(()));
        assert!(position(&docker, "docker pull") < position(&docker, "docker run"));
        assert_eq!(count(&docker, "docker pull"), 1);
        assert_eq!(count(&docker, "docker run --init"), 2);
        assert_eq!(
            docker.container_image(STAX).as_deref(),
            Some("sha256:image2")
        );
        assert_eq!(
            docker.container_image(FLEX).as_deref(),
            Some("sha256:image2")
        );
    }

    #[tokio::test]
    async fn creating_uses_the_image_that_is_there_without_pulling() {
        let docker = FakeDocker::new().with_container(STAX, "running");
        let manager = ContainerManager::new(&docker, config());

        let result = manager
            .run_container_action(ContainerAction::Create, &[Device::Stax], NOW)
            .await;

        assert_eq!(result, Ok(()));
        assert_eq!(count(&docker, "docker pull"), 0);
        assert_eq!(count(&docker, "docker run --init"), 1);
        assert_eq!(docker.container_status(STAX), Some("running"));
    }

    #[tokio::test]
    async fn creating_without_an_image_is_an_error_and_changes_nothing() {
        let docker = FakeDocker::new()
            .without_image()
            .with_container(STAX, "running");
        let manager = ContainerManager::new(&docker, config());

        let result = manager
            .run_container_action(ContainerAction::Create, &[Device::Stax], NOW)
            .await;

        assert!(result.unwrap_err().contains("update the containers"));
        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn without_docker_a_container_action_is_an_error() {
        let docker = FakeDocker::new().docker_down();
        let manager = ContainerManager::new(&docker, config());

        let result = manager
            .run_container_action(ContainerAction::Update, &[Device::Stax], NOW)
            .await;

        assert!(result.unwrap_err().contains("Docker is not available"));
        assert_eq!(mutations(&docker), 0);
    }

    #[tokio::test]
    async fn a_container_with_an_active_run_is_not_replaced() {
        let docker = FakeDocker::new()
            .with_container(STAX, "running")
            .with_container(FLEX, "running");
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);
        let _active = manager
            .runs()
            .begin(Device::Stax, RunId::new("run1").unwrap());

        let result = manager
            .run_container_action(ContainerAction::Create, &[Device::Stax, Device::Flex], NOW)
            .await;

        assert_eq!(result, Ok(()));
        assert_eq!(count(&docker, "docker run --init"), 1, "only the idle one");
        let notes: Vec<_> = drain(&mut receiver)
            .into_iter()
            .filter_map(|event| match event {
                LogEvent::Note { source, text } => Some((source, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            notes,
            [(
                Source::Container(Device::Stax),
                "not replaced: a run is active in this container".to_owned()
            )]
        );
    }

    fn collect_step() -> InContainer {
        task::collect_tests("./tests", Device::Stax)
    }

    #[tokio::test]
    async fn a_step_in_a_container_answers_with_what_it_printed_and_leaves_nothing_behind() {
        let docker = FakeDocker::new().with_exec_output("--collect-only", "test_a.py::test_x\n");
        let (sender, mut receiver) = log::channel();
        let manager = ContainerManager::new(&docker, config()).with_logs(sender);

        let output = manager
            .capture(Device::Stax, &collect_step(), Duration::from_secs(5))
            .await;

        assert_eq!(output, Ok("test_a.py::test_x\n".to_owned()));
        assert!(drain(&mut receiver).is_empty(), "a question is not logged");
        let calls = docker.calls();
        assert_eq!(calls.len(), 2, "the step, then the cleanup: {calls:?}");
        assert!(calls[1].contains("docker exec --user 0"), "{calls:?}");
        assert!(!manager.runs().is_busy(Device::Stax));
    }

    #[tokio::test]
    async fn a_failing_step_answers_with_the_end_of_its_output() {
        let docker = FakeDocker::new().fail("--collect-only", 1);
        let manager = ContainerManager::new(&docker, config());

        let error = manager
            .capture(Device::Stax, &collect_step(), Duration::from_secs(5))
            .await
            .unwrap_err();

        assert!(error.contains("injected failure"), "{error}");
    }

    #[tokio::test]
    async fn a_cancelled_plan_starts_nothing_and_cleans_up_the_container() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let plan = plan_for(
            Task::BuildApp,
            &[Device::Stax, Device::Flex],
            &Options::default(),
        );
        let cancel = CancelToken::new();
        cancel.cancel();

        let results = manager.run_plan_until(&plan, &cancel).await;

        assert!(
            results
                .iter()
                .all(|result| result.outcome == JobOutcome::Cancelled)
        );
        let calls = docker.calls();
        assert_eq!(
            calls.len(),
            2,
            "only the cleanup of each container: {calls:?}"
        );
        assert!(
            calls
                .iter()
                .all(|call| call.contains("docker exec --user 0")),
            "{calls:?}"
        );
        assert!(!manager.runs().is_busy(Device::Stax));
    }

    #[tokio::test]
    async fn a_host_job_that_is_cancelled_has_nothing_to_clean_up_in_a_container() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let plan = host_plan(&[&["python3", "-V"]]);
        let cancel = CancelToken::new();
        cancel.cancel();

        let results = manager.run_plan_until(&plan, &cancel).await;

        assert_eq!(results[0].outcome, JobOutcome::Cancelled);
        assert!(docker.calls().is_empty());
    }

    #[tokio::test]
    async fn a_plan_that_is_not_cancelled_runs_to_its_end() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());
        let plan = plan_for(Task::BuildApp, &[Device::Stax], &Options::default());

        let results = manager.run_plan_until(&plan, &CancelToken::new()).await;

        assert_eq!(results[0].outcome, JobOutcome::Succeeded);
    }

    const STAX_CACHE: &str = "ledger-pip-cache-1000";

    #[tokio::test]
    async fn a_container_gets_the_pip_cache_of_its_user_which_is_given_to_that_user_first() {
        let docker = FakeDocker::new();
        let manager = ContainerManager::new(&docker, config());

        manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(docker.volumes(), [STAX_CACHE]);
        assert!(
            position(&docker, "docker volume create") < position(&docker, "docker run --rm"),
            "the volume exists before it is given away"
        );
        assert!(
            position(&docker, "docker run --rm") < position(&docker, "docker run --init"),
            "the user owns it before the container uses it"
        );
        assert_eq!(docker.container_cache(STAX).as_deref(), Some(STAX_CACHE));
    }

    #[tokio::test]
    async fn root_has_the_original_cache_and_nothing_to_give_away() {
        let docker = FakeDocker::new();
        let config = Config {
            user: ContainerUser::Root,
            ..config()
        };
        let manager = ContainerManager::new(&docker, config);

        manager.ensure_ready(&[Device::Stax], NOW).await;

        assert_eq!(docker.volumes(), ["ledger-pip-cache"]);
        assert_eq!(count(&docker, "docker run --rm"), 0);
        assert_eq!(
            docker.container_cache(STAX).as_deref(),
            Some("ledger-pip-cache")
        );
    }

    #[tokio::test]
    async fn a_container_that_mounts_the_old_cache_is_replaced_with_auto_update_and_kept_without() {
        let old = || FakeDocker::new().with_container_cache(STAX, "running", "ledger-pip-cache");
        let (kept, replaced) = (old(), old());

        ContainerManager::new(&kept, config())
            .ensure_ready(&[Device::Stax], NOW)
            .await;
        ContainerManager::new(&replaced, auto())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(count(&kept, "docker rm"), 0, "never without auto update");
        assert_eq!(
            kept.container_cache(STAX).as_deref(),
            Some("ledger-pip-cache")
        );
        assert_eq!(count(&replaced, "docker rm"), 1);
        assert_eq!(replaced.container_cache(STAX).as_deref(), Some(STAX_CACHE));
    }

    #[tokio::test]
    async fn a_container_that_already_has_the_cache_is_left_alone() {
        let docker = FakeDocker::new().with_container(STAX, "running");

        ContainerManager::new(&docker, auto())
            .ensure_ready(&[Device::Stax], NOW)
            .await;

        assert_eq!(mutations(&docker), 0);
    }
}
