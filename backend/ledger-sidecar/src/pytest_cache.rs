//! What pytest remembers about the tests that failed, read from the app folder.

use std::collections::BTreeMap;
use std::path::Path;

use ledger_core::device::Device;
use ledger_core::failures;

/// The ids of the tests that failed their last run, for each of these devices. A device without
/// a readable cache, because no test ran on it yet, is not in the answer.
pub async fn read_failed(folder: &Path, devices: &[Device]) -> BTreeMap<Device, Vec<String>> {
    let mut failed = BTreeMap::new();
    for &device in devices {
        let file = folder.join(failures::last_failed_file(device));
        let ids = tokio::fs::read_to_string(&file)
            .await
            .ok()
            .and_then(|text| failures::parse_last_failed(&text).ok());
        if let Some(ids) = ids {
            failed.insert(device, ids);
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(folder: &Path, device: Device, text: &str) {
        let file = folder.join(failures::last_failed_file(device));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("pytest-cache-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[tokio::test]
    async fn each_device_is_read_from_its_own_cache() {
        let folder = scratch("each");
        write(&folder, Device::Stax, r#"{"t.py::a": true}"#);
        write(
            &folder,
            Device::Flex,
            r#"{"t.py::b": true, "t.py::c": true}"#,
        );

        let failed = read_failed(&folder, &[Device::Stax, Device::Flex, Device::NanoX]).await;

        assert_eq!(failed[&Device::Stax], ["t.py::a"]);
        assert_eq!(failed[&Device::Flex], ["t.py::b", "t.py::c"]);
        assert!(!failed.contains_key(&Device::NanoX), "nothing ran on it");
    }

    #[tokio::test]
    async fn a_cache_that_is_not_json_counts_as_no_cache() {
        let folder = scratch("broken");
        write(&folder, Device::Stax, "not json");

        assert!(read_failed(&folder, &[Device::Stax]).await.is_empty());
    }
}
