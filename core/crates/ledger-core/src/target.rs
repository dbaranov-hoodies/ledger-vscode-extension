use std::collections::BTreeSet;

use crate::device::Device;

/// The devices an action runs for: a non-empty choice among the devices the app supports.
///
/// Actions that need exactly one device (emulator, physical device, terminal) require
/// [`Target::single`]. The others run once per device in [`Target::devices`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    devices: BTreeSet<Device>,
}

/// Result of checking a [`Target`] against the devices an app supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub target: Target,
    /// Requested devices the app does not support, so they are not in `target`.
    pub unsupported: BTreeSet<Device>,
}

impl Target {
    /// Label of a target covering every device the app supports, and the legacy setting value.
    pub const ALL_LABEL: &'static str = "All";

    /// `None` when there is no device: there is nothing to run for.
    pub fn new(devices: impl IntoIterator<Item = Device>) -> Option<Self> {
        let devices: BTreeSet<_> = devices.into_iter().collect();
        (!devices.is_empty()).then_some(Self { devices })
    }

    pub fn one(device: Device) -> Self {
        Self {
            devices: BTreeSet::from([device]),
        }
    }

    /// Every device of `compatible`. `None` when the app supports no device.
    pub fn all(compatible: &[Device]) -> Option<Self> {
        Self::new(compatible.iter().copied())
    }

    /// The selected devices, in the order of [`Device::ALL`].
    pub fn devices(&self) -> &BTreeSet<Device> {
        &self.devices
    }

    /// The device, when exactly one is selected.
    pub fn single(&self) -> Option<Device> {
        match self.devices.len() {
            1 => self.devices.first().copied(),
            _ => None,
        }
    }

    pub fn is_multiple(&self) -> bool {
        self.devices.len() > 1
    }

    /// Text for the status bar and the UI: `All`, or the device names.
    pub fn label(&self, compatible: &[Device]) -> String {
        let supported: BTreeSet<Device> = compatible.iter().copied().collect();
        if self.is_multiple() && self.devices == supported {
            return Self::ALL_LABEL.to_owned();
        }
        let names: Vec<_> = self
            .devices
            .iter()
            .map(|device| device.spec().display_name)
            .collect();
        names.join(", ")
    }

    /// Reads a value of the existing `selectedDevice` setting: `All` or a device display name.
    pub fn from_legacy_setting(value: &str, compatible: &[Device]) -> Option<Self> {
        if value == Self::ALL_LABEL {
            Self::all(compatible)
        } else {
            Device::from_display_name(value).map(Self::one)
        }
    }

    /// Keeps the devices the app supports. If none is left, falls back to the app's first
    /// device. Returns `None` when the app supports no device.
    pub fn resolve(&self, compatible: &[Device]) -> Option<Resolution> {
        let supported: BTreeSet<Device> = compatible.iter().copied().collect();
        let kept = Self::new(self.devices.intersection(&supported).copied());
        let target = kept.or_else(|| compatible.first().copied().map(Self::one))?;
        Some(Resolution {
            target,
            unsupported: self.devices.difference(&supported).copied().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE: [Device; 5] = [
        Device::NanoX,
        Device::NanoSPlus,
        Device::Stax,
        Device::Flex,
        Device::ApexP,
    ];

    fn target(devices: &[Device]) -> Target {
        Target::new(devices.iter().copied()).expect("at least one device")
    }

    fn set(devices: &[Device]) -> BTreeSet<Device> {
        devices.iter().copied().collect()
    }

    #[test]
    fn a_target_is_never_empty() {
        assert_eq!(Target::new(std::iter::empty()), None);
        assert_eq!(Target::all(&[]), None);
    }

    #[test]
    fn the_same_devices_make_the_same_target_whatever_the_order_or_repeats() {
        assert_eq!(
            target(&[Device::Flex, Device::NanoX]),
            target(&[Device::NanoX, Device::Flex, Device::NanoX])
        );
    }

    #[test]
    fn devices_come_back_in_the_device_list_order() {
        let devices: Vec<_> = target(&[Device::ApexP, Device::NanoX, Device::Flex])
            .devices()
            .iter()
            .copied()
            .collect();

        assert_eq!(devices, [Device::NanoX, Device::Flex, Device::ApexP]);
    }

    #[test]
    fn one_device_is_single() {
        let target = Target::one(Device::Stax);

        assert_eq!(target.single(), Some(Device::Stax));
        assert!(!target.is_multiple());
    }

    #[test]
    fn several_devices_are_not_single() {
        let target = target(&[Device::NanoX, Device::NanoSPlus]);

        assert_eq!(target.single(), None);
        assert!(target.is_multiple());
    }

    #[test]
    fn all_selects_every_supported_device() {
        let all = Target::all(&FIVE).unwrap();

        assert_eq!(all.devices(), &set(&FIVE));
    }

    #[test]
    fn resolve_keeps_a_fully_supported_choice() {
        let resolution = target(&[Device::Flex, Device::ApexP])
            .resolve(&FIVE)
            .unwrap();

        assert_eq!(resolution.target, target(&[Device::Flex, Device::ApexP]));
        assert!(resolution.unsupported.is_empty());
    }

    #[test]
    fn resolve_drops_unsupported_devices_and_reports_them() {
        let resolution = target(&[Device::NanoS, Device::Stax])
            .resolve(&FIVE)
            .unwrap();

        assert_eq!(resolution.target, Target::one(Device::Stax));
        assert_eq!(resolution.unsupported, set(&[Device::NanoS]));
    }

    #[test]
    fn resolve_falls_back_to_the_first_supported_device() {
        let resolution = Target::one(Device::NanoS).resolve(&FIVE).unwrap();

        assert_eq!(resolution.target, Target::one(Device::NanoX));
        assert_eq!(resolution.unsupported, set(&[Device::NanoS]));
    }

    #[test]
    fn resolve_finds_nothing_when_the_app_supports_no_device() {
        assert_eq!(Target::one(Device::Stax).resolve(&[]), None);
    }

    #[test]
    fn label_is_all_only_for_the_full_supported_set() {
        assert_eq!(Target::all(&FIVE).unwrap().label(&FIVE), "All");
        assert_eq!(
            target(&[Device::Flex, Device::ApexP]).label(&FIVE),
            "Flex, Apex p"
        );
        assert_eq!(Target::one(Device::Stax).label(&FIVE), "Stax");
    }

    #[test]
    fn a_one_device_app_labels_its_device_not_all() {
        let only_stax = [Device::Stax];

        assert_eq!(Target::all(&only_stax).unwrap().label(&only_stax), "Stax");
    }

    #[test]
    fn legacy_settings_are_read_back() {
        assert_eq!(
            Target::from_legacy_setting("All", &FIVE),
            Target::all(&FIVE)
        );
        assert_eq!(
            Target::from_legacy_setting("Nano S Plus", &FIVE),
            Some(Target::one(Device::NanoSPlus))
        );
        assert_eq!(Target::from_legacy_setting("all", &FIVE), None);
        assert_eq!(Target::from_legacy_setting("Nano", &FIVE), None);
    }
}
