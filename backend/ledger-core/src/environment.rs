//! What the core cannot know by itself: the settings, what Docker and the tools report, and what
//! is running right now. The caller gathers it and passes it in.

use std::collections::BTreeSet;

use crate::actions::ActionId;
use crate::container::Observation;
use crate::device_ops::DeviceSettings;
use crate::docker::ContainerUser;
use crate::failures::Failures;
use crate::host_python::HostPython;
use crate::test_tree::TestTree;

#[derive(Debug, Clone)]
pub struct Environment {
    pub settings: Settings,
    /// Docker and the containers of the app, one per device.
    pub containers: Observation,
    pub user: ContainerUser,
    pub host: HostPython,
    /// The first service of the compose file of the app, once it was asked.
    pub compose_service: Option<String>,
    /// The tests of the selected suite, once they were collected.
    pub test_tree: Option<TestTree>,
    pub collecting_tests: bool,
    /// The tests of that list that failed their last run, on each device.
    pub failures: Failures,
    /// The selected suite ships the scripts that prepare its dependencies.
    pub test_helpers: bool,
    pub running: BTreeSet<ActionId>,
    pub ai_models: Vec<String>,
    pub ai_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub verbose_tests: bool,
    /// A shell command to run before the tests, to install what they need.
    pub test_requirements: String,
    pub device: DeviceSettings,
}
