//! The development containers of an app, one per device, and what to do to make them ready.

use serde::Serialize;
use ts_rs::TS;

use crate::device::Device;
use crate::update::Freshness;

/// Name of the container of an app for one device: `<app folder>-container-<device>`.
pub fn container_name(app_folder: &str, device: Device) -> String {
    format!("{app_folder}-container-{}", device.spec().name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ContainerState {
    Missing,
    Stopped,
    Starting,
    Running,
}

impl ContainerState {
    /// Reads a Docker status such as `running`, the `State.Status` of `docker inspect`.
    pub fn from_docker_status(status: &str) -> Self {
        match status.trim() {
            "running" => Self::Running,
            "restarting" | "starting" => Self::Starting,
            _ => Self::Stopped,
        }
    }
}

/// What `docker inspect` says of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspected {
    pub state: ContainerState,
    /// The image the container was created from.
    pub image_id: Option<String>,
    /// The volume mounted as its pip cache.
    pub pip_cache: Option<String>,
}

/// Reads the output of [`crate::docker::container_status`].
pub fn parse_inspect(output: &str) -> Inspected {
    let mut words = output.split_whitespace();
    let state = ContainerState::from_docker_status(words.next().unwrap_or(""));
    Inspected {
        state,
        image_id: words.next().map(str::to_owned),
        pip_cache: words.next().map(str::to_owned),
    }
}

/// How ready the containers of a target are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Status {
    Running,
    /// No container is down, but some is still starting.
    Syncing,
    /// A container is missing or stopped, or there is none.
    Stopped,
}

pub fn aggregate(states: impl IntoIterator<Item = ContainerState>) -> Status {
    let mut any = false;
    let mut starting = false;
    for state in states {
        any = true;
        match state {
            ContainerState::Missing | ContainerState::Stopped => return Status::Stopped,
            ContainerState::Starting => starting = true,
            ContainerState::Running => {}
        }
    }
    match (any, starting) {
        (false, _) => Status::Stopped,
        (true, true) => Status::Syncing,
        (true, false) => Status::Running,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedContainer {
    pub device: Device,
    pub state: ContainerState,
    /// The image the container was created from.
    pub image_id: Option<String>,
    /// It mounts the pip cache that its user can write to. Without it, every container downloads
    /// and builds the packages of the tests again.
    pub pip_cache_ready: bool,
    /// A run is active in it, so it must not be replaced.
    pub busy: bool,
}

/// What Docker reports about the containers of a target, plus what is known of the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub docker_available: bool,
    /// The id of the local image, when it is there.
    pub image_id: Option<String>,
    pub freshness: Freshness,
    pub containers: Vec<ObservedContainer>,
}

impl Observation {
    pub fn image_present(&self) -> bool {
        self.image_id.is_some()
    }

    pub fn status(&self) -> Status {
        aggregate(self.containers.iter().map(|container| container.state))
    }

    pub fn state_of(&self, device: Device) -> ContainerState {
        self.containers
            .iter()
            .find(|container| container.device == device)
            .map_or(ContainerState::Missing, |container| container.state)
    }

    /// The container should be created again: it comes from an older image than the local one,
    /// or it lacks the pip cache that it needs.
    pub fn is_stale(&self, container: &ObservedContainer) -> bool {
        container.state != ContainerState::Missing
            && (!container.pip_cache_ready
                || matches!(
                    (&container.image_id, &self.image_id),
                    (Some(created_from), Some(local)) if created_from != local
                ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Replace containers that run an old image, and pull when the registry has a newer one.
    pub auto_update: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Pull,
    /// Starts an existing container.
    Start(Device),
    /// Removes the container if it exists, then creates and starts a new one.
    Create(Device),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    DockerUnavailable,
    /// The containers are ready and nothing is worth updating.
    Nothing,
    Steps {
        operations: Vec<Operation>,
        /// Containers that should be replaced but have a run active: for later.
        deferred: Vec<Device>,
    },
}

/// Decides what to do to make the containers of a target ready.
///
/// Updating is careful: it never happens without `auto_update`, never on a registry that was
/// not reachable, never replaces a container that has a run active, and always pulls before
/// replacing anything.
pub fn plan(observation: &Observation, settings: &Settings) -> Plan {
    if !observation.docker_available {
        return Plan::DockerUnavailable;
    }
    if observation.containers.is_empty() {
        return Plan::Nothing;
    }

    let registry_newer = settings.auto_update && observation.freshness == Freshness::Outdated;
    let mut operations = Vec::new();
    let mut deferred = Vec::new();
    for container in &observation.containers {
        let exists = container.state != ContainerState::Missing;
        let outdated =
            settings.auto_update && exists && (registry_newer || observation.is_stale(container));
        match (outdated, container.busy) {
            (true, true) => deferred.push(container.device),
            (true, false) => operations.push(Operation::Create(container.device)),
            (false, _) => match container.state {
                ContainerState::Missing => operations.push(Operation::Create(container.device)),
                ContainerState::Stopped => operations.push(Operation::Start(container.device)),
                ContainerState::Starting | ContainerState::Running => {}
            },
        }
    }

    let creates = operations
        .iter()
        .any(|operation| matches!(operation, Operation::Create(_)));
    if registry_newer || (creates && !observation.image_present()) {
        operations.insert(0, Operation::Pull);
    }

    if operations.is_empty() && deferred.is_empty() {
        Plan::Nothing
    } else {
        Plan::Steps {
            operations,
            deferred,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// The containers are ready.
    Done,
    Again,
    GiveUp,
}

/// Limits how many times making the containers ready is retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retries {
    max: u32,
    used: u32,
}

impl Retries {
    pub const fn new(max: u32) -> Self {
        Self { max, used: 0 }
    }

    /// Call when an attempt has ended, with whether the containers are ready.
    pub fn attempt_ended(&mut self, ready: bool) -> Retry {
        if ready {
            self.used = 0;
            Retry::Done
        } else if self.used < self.max {
            self.used += 1;
            Retry::Again
        } else {
            self.used = 0;
            Retry::GiveUp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ContainerState::{Missing, Running, Starting, Stopped};
    use super::*;

    const LOCAL: &str = "sha256:new";
    const OLD: &str = "sha256:old";

    fn container(
        device: Device,
        state: ContainerState,
        image: &str,
        busy: bool,
    ) -> ObservedContainer {
        ObservedContainer {
            device,
            state,
            image_id: (state != Missing).then(|| image.to_owned()),
            pip_cache_ready: true,
            busy,
        }
    }

    fn observe(
        image_present: bool,
        freshness: Freshness,
        containers: Vec<ObservedContainer>,
    ) -> Observation {
        Observation {
            docker_available: true,
            image_id: image_present.then(|| LOCAL.to_owned()),
            freshness,
            containers,
        }
    }

    /// Containers on the current image, none of them busy.
    fn current(containers: &[(Device, ContainerState)]) -> Vec<ObservedContainer> {
        containers
            .iter()
            .map(|&(device, state)| container(device, state, LOCAL, false))
            .collect()
    }

    fn steps(operations: Vec<Operation>) -> Plan {
        Plan::Steps {
            operations,
            deferred: Vec::new(),
        }
    }

    const MANUAL: Settings = Settings { auto_update: false };
    const AUTO: Settings = Settings { auto_update: true };
    const UNKNOWN: Freshness = Freshness::Unknown;

    #[test]
    fn container_names_end_with_the_device_name() {
        assert_eq!(
            container_name("app-boilerplate", Device::NanoSPlus),
            "app-boilerplate-container-nanosp"
        );
    }

    #[test]
    fn docker_statuses_map_to_container_states() {
        assert_eq!(ContainerState::from_docker_status("running\n"), Running);
        assert_eq!(ContainerState::from_docker_status("restarting"), Starting);
        assert_eq!(ContainerState::from_docker_status("exited"), Stopped);
        assert_eq!(ContainerState::from_docker_status("created"), Stopped);
        assert_eq!(ContainerState::from_docker_status("paused"), Stopped);
    }

    #[test]
    fn the_inspect_output_gives_the_state_the_image_and_the_pip_cache() {
        let inspected = |state, image: Option<&str>, cache: Option<&str>| Inspected {
            state,
            image_id: image.map(str::to_owned),
            pip_cache: cache.map(str::to_owned),
        };

        assert_eq!(
            parse_inspect("running sha256:abc ledger-pip-cache-1000\n"),
            inspected(Running, Some("sha256:abc"), Some("ledger-pip-cache-1000"))
        );
        assert_eq!(
            parse_inspect("running sha256:abc \n"),
            inspected(Running, Some("sha256:abc"), None),
            "a container made before the cache had a volume of its own"
        );
        assert_eq!(parse_inspect("exited"), inspected(Stopped, None, None));
        assert_eq!(parse_inspect(""), inspected(Stopped, None, None));
    }

    #[test]
    fn a_container_without_the_pip_cache_is_replaced_like_one_on_an_old_image() {
        let mut without_cache = container(Device::Stax, Running, LOCAL, false);
        without_cache.pip_cache_ready = false;
        let observed = observe(true, UNKNOWN, vec![without_cache.clone()]);

        assert_eq!(
            plan(&observed, &AUTO),
            steps(vec![Operation::Create(Device::Stax)])
        );
        assert_eq!(
            plan(&observed, &MANUAL),
            Plan::Nothing,
            "never without auto update"
        );
        without_cache.busy = true;
        assert_eq!(
            plan(&observe(true, UNKNOWN, vec![without_cache]), &AUTO),
            Plan::Steps {
                operations: vec![],
                deferred: vec![Device::Stax]
            },
            "a run in it is not interrupted"
        );
    }

    #[test]
    fn the_status_of_several_containers_is_the_worst_one() {
        assert_eq!(aggregate([Running, Running]), Status::Running);
        assert_eq!(aggregate([Running, Starting]), Status::Syncing);
        assert_eq!(aggregate([Starting, Stopped]), Status::Stopped);
        assert_eq!(aggregate([Running, Missing]), Status::Stopped);
        assert_eq!(aggregate([]), Status::Stopped);
    }

    #[test]
    fn nothing_is_planned_without_docker() {
        let mut observed = observe(true, UNKNOWN, current(&[(Device::Stax, Missing)]));
        observed.docker_available = false;

        assert_eq!(plan(&observed, &MANUAL), Plan::DockerUnavailable);
    }

    #[test]
    fn nothing_is_planned_without_containers_to_manage() {
        assert_eq!(plan(&observe(true, UNKNOWN, vec![]), &AUTO), Plan::Nothing);
    }

    #[test]
    fn nothing_is_planned_when_containers_run_or_start() {
        let observed = observe(
            true,
            UNKNOWN,
            current(&[(Device::Stax, Running), (Device::Flex, Starting)]),
        );

        assert_eq!(plan(&observed, &MANUAL), Plan::Nothing);
        assert_eq!(plan(&observed, &AUTO), Plan::Nothing);
    }

    #[test]
    fn stopped_containers_are_started_and_running_ones_are_left_alone() {
        let observed = observe(
            false,
            UNKNOWN,
            current(&[(Device::Stax, Stopped), (Device::Flex, Running)]),
        );

        assert_eq!(
            plan(&observed, &MANUAL),
            steps(vec![Operation::Start(Device::Stax)])
        );
    }

    #[test]
    fn missing_containers_are_created_from_the_local_image_without_pulling() {
        let observed = observe(
            true,
            UNKNOWN,
            current(&[(Device::Stax, Missing), (Device::Flex, Running)]),
        );

        assert_eq!(
            plan(&observed, &MANUAL),
            steps(vec![Operation::Create(Device::Stax)])
        );
    }

    #[test]
    fn the_image_is_pulled_first_when_it_is_missing_locally() {
        let observed = observe(
            false,
            UNKNOWN,
            current(&[(Device::Stax, Missing), (Device::Flex, Stopped)]),
        );

        assert_eq!(
            plan(&observed, &MANUAL),
            steps(vec![
                Operation::Pull,
                Operation::Create(Device::Stax),
                Operation::Start(Device::Flex)
            ])
        );
    }

    #[test]
    fn a_newer_registry_image_changes_nothing_without_auto_update() {
        let observed = observe(
            true,
            Freshness::Outdated,
            current(&[(Device::Stax, Running)]),
        );

        assert_eq!(plan(&observed, &MANUAL), Plan::Nothing);
    }

    #[test]
    fn an_unknown_or_current_registry_state_never_triggers_an_update() {
        for freshness in [Freshness::Unknown, Freshness::UpToDate] {
            let observed = observe(true, freshness, current(&[(Device::Stax, Running)]));

            assert_eq!(plan(&observed, &AUTO), Plan::Nothing);
        }
    }

    #[test]
    fn a_newer_registry_image_is_pulled_before_idle_containers_are_replaced() {
        let observed = observe(
            true,
            Freshness::Outdated,
            current(&[(Device::Stax, Running), (Device::Flex, Stopped)]),
        );

        assert_eq!(
            plan(&observed, &AUTO),
            steps(vec![
                Operation::Pull,
                Operation::Create(Device::Stax),
                Operation::Create(Device::Flex)
            ])
        );
    }

    #[test]
    fn a_container_with_a_run_active_is_deferred_but_the_image_is_still_pulled() {
        let containers = vec![
            container(Device::Stax, Running, LOCAL, true),
            container(Device::Flex, Running, LOCAL, false),
        ];
        let observed = observe(true, Freshness::Outdated, containers);

        assert_eq!(
            plan(&observed, &AUTO),
            Plan::Steps {
                operations: vec![Operation::Pull, Operation::Create(Device::Flex)],
                deferred: vec![Device::Stax],
            }
        );
    }

    #[test]
    fn only_deferred_containers_leave_nothing_to_do_but_the_pull() {
        let containers = vec![container(Device::Stax, Running, LOCAL, true)];
        let observed = observe(true, Freshness::Outdated, containers);

        assert_eq!(
            plan(&observed, &AUTO),
            Plan::Steps {
                operations: vec![Operation::Pull],
                deferred: vec![Device::Stax],
            }
        );
    }

    #[test]
    fn a_container_on_an_older_local_image_is_replaced_without_pulling() {
        let containers = vec![
            container(Device::Stax, Running, OLD, false),
            container(Device::Flex, Running, LOCAL, false),
        ];
        let observed = observe(true, UNKNOWN, containers);

        assert_eq!(
            plan(&observed, &AUTO),
            steps(vec![Operation::Create(Device::Stax)])
        );
        assert_eq!(plan(&observed, &MANUAL), Plan::Nothing);
    }

    #[test]
    fn a_stale_container_that_is_busy_is_only_deferred() {
        let containers = vec![container(Device::Stax, Running, OLD, true)];
        let observed = observe(true, UNKNOWN, containers);

        assert_eq!(
            plan(&observed, &AUTO),
            Plan::Steps {
                operations: vec![],
                deferred: vec![Device::Stax],
            }
        );
    }

    #[test]
    fn a_missing_container_is_just_created_even_when_an_update_is_pending() {
        let observed = observe(
            true,
            Freshness::Outdated,
            current(&[(Device::Stax, Missing)]),
        );

        assert_eq!(
            plan(&observed, &AUTO),
            steps(vec![Operation::Pull, Operation::Create(Device::Stax)])
        );
    }

    #[test]
    fn retries_stop_after_the_allowed_number_of_attempts() {
        let mut retries = Retries::new(2);

        assert_eq!(retries.attempt_ended(false), Retry::Again);
        assert_eq!(retries.attempt_ended(false), Retry::Again);
        assert_eq!(retries.attempt_ended(false), Retry::GiveUp);
        assert_eq!(retries.attempt_ended(false), Retry::Again);
    }

    #[test]
    fn a_ready_attempt_resets_the_retries() {
        let mut retries = Retries::new(1);

        assert_eq!(retries.attempt_ended(false), Retry::Again);
        assert_eq!(retries.attempt_ended(true), Retry::Done);
        assert_eq!(retries.attempt_ended(false), Retry::Again);
    }
}
