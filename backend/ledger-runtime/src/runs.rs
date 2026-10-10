use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ledger_core::Device;
use ledger_core::docker::RunId;

/// The runs that are active in each device's container. A container with an active run is busy:
/// it must not be replaced, and the run can be asked to stop from anywhere.
#[derive(Debug, Clone, Default)]
pub struct ActiveRuns {
    runs: Arc<Mutex<HashMap<Device, Vec<RunId>>>>,
}

impl ActiveRuns {
    /// The run is active until the guard is dropped.
    pub fn begin(&self, device: Device, run_id: RunId) -> RunGuard {
        self.runs
            .lock()
            .unwrap()
            .entry(device)
            .or_default()
            .push(run_id.clone());
        RunGuard {
            runs: self.clone(),
            device,
            run_id,
        }
    }

    pub fn is_busy(&self, device: Device) -> bool {
        !self.ids(device).is_empty()
    }

    pub fn ids(&self, device: Device) -> Vec<RunId> {
        self.runs
            .lock()
            .unwrap()
            .get(&device)
            .cloned()
            .unwrap_or_default()
    }

    fn end(&self, device: Device, run_id: &RunId) {
        let mut runs = self.runs.lock().unwrap();
        if let Some(ids) = runs.get_mut(&device) {
            ids.retain(|id| id != run_id);
            if ids.is_empty() {
                runs.remove(&device);
            }
        }
    }
}

/// Keeps a run active while it lives.
#[derive(Debug)]
pub struct RunGuard {
    runs: ActiveRuns,
    device: Device,
    run_id: RunId,
}

impl RunGuard {
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.runs.end(self.device, &self.run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> RunId {
        RunId::new(value).unwrap()
    }

    #[test]
    fn a_device_is_busy_while_a_run_is_active() {
        let runs = ActiveRuns::default();
        assert!(!runs.is_busy(Device::Stax));

        let guard = runs.begin(Device::Stax, id("a"));

        assert!(runs.is_busy(Device::Stax));
        assert!(!runs.is_busy(Device::Flex));
        assert_eq!(runs.ids(Device::Stax), [id("a")]);
        drop(guard);
        assert!(!runs.is_busy(Device::Stax));
    }

    #[test]
    fn several_runs_on_one_device_end_independently() {
        let runs = ActiveRuns::default();
        let first = runs.begin(Device::Stax, id("a"));
        let _second = runs.begin(Device::Stax, id("b"));

        drop(first);

        assert_eq!(runs.ids(Device::Stax), [id("b")]);
        assert!(runs.is_busy(Device::Stax));
    }

    #[test]
    fn clones_share_the_same_runs() {
        let runs = ActiveRuns::default();
        let other = runs.clone();

        let _guard = other.begin(Device::Flex, id("x"));

        assert!(runs.is_busy(Device::Flex));
    }
}
