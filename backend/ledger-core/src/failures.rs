//! Which tests failed on which device. Each device keeps its own pytest cache (see
//! [`cache_dir`]), so what failed on one device is not lost when another one runs.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::IgnoredAny;

use crate::device::Device;
use crate::test_tree::{TestTree, without_parameters};

/// The directory pytest keeps its cache in for this device, relative to the app folder.
pub fn cache_dir(device: Device) -> String {
    format!(".pytest_cache/{}", device.spec().name)
}

/// The file of that cache that lists the tests that failed their last run, relative to the
/// app folder.
pub fn last_failed_file(device: Device) -> String {
    format!("{}/v/cache/lastfailed", cache_dir(device))
}

/// The ids in a `lastfailed` file: a JSON object that has the id of a failed test as a key.
pub fn parse_last_failed(text: &str) -> Result<Vec<String>, serde_json::Error> {
    let ids: BTreeMap<String, IgnoredAny> = serde_json::from_str(text)?;
    Ok(ids.into_keys().collect())
}

/// The tests of the list that failed their last run, for each device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Failures {
    /// Devices where nothing failed are not here.
    by_device: BTreeMap<Device, BTreeSet<String>>,
}

impl Failures {
    /// Matches the ids that pytest wrote in the cache of each device to the tests of the list.
    /// A variant counts as its test, and a failed directory, file or class as every test below
    /// it. Ids of tests that the list no longer has are dropped.
    pub fn from_caches(
        tree: &TestTree,
        suite_directory: &str,
        caches: &BTreeMap<Device, Vec<String>>,
    ) -> Self {
        let nodes = tree.nodes_by_run_id(suite_directory);
        let by_device = caches
            .iter()
            .filter_map(|(device, ids)| {
                let failed: BTreeSet<String> = ids
                    .iter()
                    .filter_map(|id| nodes.get(without_parameters(id)))
                    .flat_map(|node| node.tests())
                    .map(|test| test.id.clone())
                    .collect();
                (!failed.is_empty()).then_some((*device, failed))
            })
            .collect();
        Self { by_device }
    }

    /// The devices this test failed on.
    pub fn devices_of(&self, test_id: &str) -> Vec<Device> {
        self.by_device
            .iter()
            .filter(|(_, failed)| failed.contains(test_id))
            .map(|(device, _)| *device)
            .collect()
    }

    /// The tests that failed on at least one of these devices.
    pub fn on_any(&self, devices: &BTreeSet<Device>) -> Vec<String> {
        let failed: BTreeSet<&String> = self
            .by_device
            .iter()
            .filter(|(device, _)| devices.contains(device))
            .flat_map(|(_, failed)| failed)
            .collect();
        failed.into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\
test_a.py::test_one[nanox]
test_a.py::test_two
sub/test_b.py::TestFoo::test_three
sub/test_b.py::TestFoo::test_four
test_c.py::test_five
";

    fn tree() -> TestTree {
        TestTree::from_collect_output(OUTPUT)
    }

    fn caches(entries: &[(Device, &[&str])]) -> BTreeMap<Device, Vec<String>> {
        entries
            .iter()
            .map(|(device, ids)| (*device, ids.iter().map(|id| (*id).to_owned()).collect()))
            .collect()
    }

    fn set(devices: &[Device]) -> BTreeSet<Device> {
        devices.iter().copied().collect()
    }

    #[test]
    fn the_cache_of_a_device_is_its_own_directory_in_the_app_folder() {
        assert_eq!(cache_dir(Device::NanoSPlus), ".pytest_cache/nanosp");
        assert_eq!(
            last_failed_file(Device::Stax),
            ".pytest_cache/stax/v/cache/lastfailed"
        );
    }

    #[test]
    fn the_failed_ids_are_the_keys_of_the_last_failed_file() {
        let text = r#"{"tests/test_a.py::test_one[nanox]": true, "tests/test_c.py": true}"#;
        assert_eq!(
            parse_last_failed(text).unwrap(),
            ["tests/test_a.py::test_one[nanox]", "tests/test_c.py"]
        );
        assert_eq!(parse_last_failed("{}").unwrap(), Vec::<String>::new());
        assert!(parse_last_failed("not json").is_err());
        assert!(parse_last_failed("[]").is_err());
    }

    #[test]
    fn ids_are_matched_whether_pytest_ran_in_the_suite_or_in_the_app() {
        let failures = Failures::from_caches(
            &tree(),
            "./tests/standalone/",
            &caches(&[
                (Device::Stax, &["test_a.py::test_two"]),
                (Device::Flex, &["tests/standalone/test_a.py::test_two"]),
            ]),
        );

        assert_eq!(
            failures.devices_of("test_a.py::test_two"),
            [Device::Stax, Device::Flex]
        );
    }

    #[test]
    fn a_failed_variant_is_its_test_and_a_failed_file_or_class_is_all_its_tests() {
        let failures = Failures::from_caches(
            &tree(),
            "./tests/",
            &caches(&[(
                Device::Stax,
                &[
                    "test_a.py::test_one[nanox-1]",
                    "sub/test_b.py::TestFoo",
                    "test_c.py",
                ],
            )]),
        );

        let failed = failures.on_any(&set(&[Device::Stax]));
        assert_eq!(
            failed,
            [
                "sub/test_b.py::TestFoo::test_four",
                "sub/test_b.py::TestFoo::test_three",
                "test_a.py::test_one",
                "test_c.py::test_five",
            ]
        );
    }

    #[test]
    fn ids_of_tests_that_are_gone_are_dropped() {
        let failures = Failures::from_caches(
            &tree(),
            "./tests/",
            &caches(&[(
                Device::Stax,
                &["test_removed.py::test_x", "test_a.py::test_gone"],
            )]),
        );

        assert_eq!(failures, Failures::default());
    }

    #[test]
    fn each_device_keeps_its_own_failures() {
        // The case of the user: target 1 failed a, b and c; target 2 c and d; target 3 a, c and d.
        let tree = TestTree::from_collect_output("t.py::a\nt.py::b\nt.py::c\nt.py::d\n");
        let failures = Failures::from_caches(
            &tree,
            "./",
            &caches(&[
                (Device::NanoX, &["t.py::a", "t.py::b", "t.py::c"]),
                (Device::Stax, &["t.py::c", "t.py::d"]),
                (Device::Flex, &["t.py::a", "t.py::c", "t.py::d"]),
            ]),
        );

        assert_eq!(
            failures.devices_of("t.py::a"),
            [Device::NanoX, Device::Flex]
        );
        assert_eq!(failures.devices_of("t.py::b"), [Device::NanoX]);
        assert_eq!(
            failures.devices_of("t.py::c"),
            [Device::NanoX, Device::Stax, Device::Flex]
        );
        assert_eq!(failures.devices_of("t.py::d"), [Device::Stax, Device::Flex]);
        assert_eq!(failures.devices_of("t.py::unknown"), Vec::<Device>::new());
    }

    #[test]
    fn the_failures_of_some_devices_are_the_union_of_theirs() {
        let tree = TestTree::from_collect_output("t.py::a\nt.py::b\nt.py::c\nt.py::d\n");
        let failures = Failures::from_caches(
            &tree,
            "./",
            &caches(&[
                (Device::NanoX, &["t.py::a", "t.py::b"]),
                (Device::Stax, &["t.py::b", "t.py::d"]),
            ]),
        );

        assert_eq!(
            failures.on_any(&set(&[Device::NanoX])),
            ["t.py::a", "t.py::b"]
        );
        assert_eq!(
            failures.on_any(&set(&[Device::NanoX, Device::Stax])),
            ["t.py::a", "t.py::b", "t.py::d"]
        );
        assert!(failures.on_any(&set(&[Device::Flex])).is_empty());
    }
}
