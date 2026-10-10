use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::language::Language;

/// A Ledger device model. Its facts live in [`Device::spec`]. Devices are ordered as declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Device {
    NanoS,
    NanoSPlus,
    NanoX,
    Stax,
    Flex,
    ApexP,
    ApexM,
}

/// Mirrors the device list of `ledgered`, Ledger's own tooling library: one canonical name per
/// device, plus the few places where a tool spells the device differently.
#[derive(Debug, PartialEq, Eq)]
pub struct DeviceSpec {
    /// Canonical name: Speculos `--model`, ragger `--device`, container suffix, protocol id.
    pub name: &'static str,
    /// Name shown in the UI and stored in the existing extension settings.
    pub display_name: &'static str,
    /// Two or three letters for tight places, such as a mark next to a test.
    pub short_name: &'static str,
    /// Further names accepted when reading a manifest.
    pub aliases: &'static [&'static str],
    /// Directory under `build/`: the C SDK target name, also what the enforcer gets for C apps.
    pub build_dir_name: &'static str,
    /// Name `cargo ledger build` expects, also what the enforcer gets for Rust apps.
    pub cargo_ledger_name: &'static str,
    /// Target id used by `ledgerblue` to delete an app from a physical device.
    pub target_id: u32,
}

impl DeviceSpec {
    /// A device that every tool calls by its canonical name.
    const fn standard(
        name: &'static str,
        display_name: &'static str,
        short_name: &'static str,
        target_id: u32,
    ) -> Self {
        Self {
            name,
            display_name,
            short_name,
            aliases: &[],
            build_dir_name: name,
            cargo_ledger_name: name,
            target_id,
        }
    }
}

const NANO_S: DeviceSpec = DeviceSpec::standard("nanos", "Nano S", "NS", 0x3110_0004);
const NANO_S_PLUS: DeviceSpec = DeviceSpec {
    aliases: &["nanos+", "nanos2", "nanosplus"],
    build_dir_name: "nanos2",
    cargo_ledger_name: "nanosplus",
    ..DeviceSpec::standard("nanosp", "Nano S Plus", "NS+", 0x3310_0004)
};
const NANO_X: DeviceSpec = DeviceSpec::standard("nanox", "Nano X", "NX", 0x3300_0004);
const STAX: DeviceSpec = DeviceSpec::standard("stax", "Stax", "ST", 0x3320_0004);
const FLEX: DeviceSpec = DeviceSpec::standard("flex", "Flex", "FX", 0x3330_0004);
const APEX_P: DeviceSpec = DeviceSpec::standard("apex_p", "Apex p", "AP", 0x3340_0004);
const APEX_M: DeviceSpec = DeviceSpec::standard("apex_m", "Apex m", "AM", 0x3350_0004);

impl Device {
    pub const ALL: [Self; 7] = [
        Self::NanoS,
        Self::NanoSPlus,
        Self::NanoX,
        Self::Stax,
        Self::Flex,
        Self::ApexP,
        Self::ApexM,
    ];

    pub const fn spec(self) -> &'static DeviceSpec {
        match self {
            Self::NanoS => &NANO_S,
            Self::NanoSPlus => &NANO_S_PLUS,
            Self::NanoX => &NANO_X,
            Self::Stax => &STAX,
            Self::Flex => &FLEX,
            Self::ApexP => &APEX_P,
            Self::ApexM => &APEX_M,
        }
    }

    /// Finds a device by canonical name or alias, ignoring case, like `ledgered` does.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|device| {
            let spec = device.spec();
            spec.name.eq_ignore_ascii_case(name)
                || spec
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(name))
        })
    }

    pub fn from_display_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|device| device.spec().display_name == name)
    }

    /// Container environment variable holding the SDK path, without the `$` (`NANOSP_SDK`).
    pub fn sdk_env_var(self) -> String {
        format!("{}_SDK", self.spec().name.to_uppercase())
    }

    /// The device name the build and check tooling expects for an app written in `language`.
    pub const fn sdk_model(self, language: Language) -> &'static str {
        let spec = self.spec();
        match language {
            Language::C => spec.build_dir_name,
            Language::Rust => spec.cargo_ledger_name,
        }
    }
}

impl Serialize for Device {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.spec().name)
    }
}

impl<'de> Deserialize<'de> for Device {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|device| device.spec().name == name)
            .ok_or_else(|| D::Error::custom(format!("unknown device `{name}`")))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn specs() -> impl Iterator<Item = &'static DeviceSpec> {
        Device::ALL.into_iter().map(Device::spec)
    }

    fn all_distinct<T: std::hash::Hash + Eq>(values: impl Iterator<Item = T>) -> bool {
        let mut seen = HashSet::new();
        values.into_iter().all(|value| seen.insert(value))
    }

    #[test]
    fn every_lookup_key_belongs_to_one_device() {
        let keys = specs().flat_map(|spec| {
            std::iter::once(spec.name)
                .chain(spec.aliases.iter().copied())
                .map(str::to_lowercase)
        });

        assert!(all_distinct(keys));
    }

    #[test]
    fn other_per_device_facts_are_unique() {
        assert!(all_distinct(specs().map(|spec| spec.display_name)));
        assert!(all_distinct(specs().map(|spec| spec.short_name)));
        assert!(all_distinct(specs().map(|spec| spec.build_dir_name)));
        assert!(all_distinct(specs().map(|spec| spec.cargo_ledger_name)));
        assert!(all_distinct(specs().map(|spec| spec.target_id)));
    }

    #[test]
    fn every_name_and_alias_finds_its_device() {
        for device in Device::ALL {
            let spec = device.spec();
            for name in std::iter::once(&spec.name).chain(spec.aliases) {
                assert_eq!(Device::from_name(name), Some(device), "{name}");
            }
        }
    }

    #[test]
    fn name_lookup_ignores_case_and_rejects_unknown_names() {
        assert_eq!(Device::from_name("NanoSP"), Some(Device::NanoSPlus));
        assert_eq!(Device::from_name("nanos"), Some(Device::NanoS));
        assert_eq!(Device::from_name("nano"), None);
        assert_eq!(Device::from_name(""), None);
    }

    #[test]
    fn display_names_round_trip() {
        for device in Device::ALL {
            assert_eq!(
                Device::from_display_name(device.spec().display_name),
                Some(device)
            );
        }
        assert_eq!(Device::from_display_name("nano s"), None);
    }

    #[test]
    fn only_nano_s_plus_is_spelled_differently_by_the_tools() {
        for device in Device::ALL.into_iter().filter(|d| *d != Device::NanoSPlus) {
            let spec = device.spec();
            assert!(spec.aliases.is_empty(), "{}", spec.name);
            assert_eq!(spec.build_dir_name, spec.name);
            assert_eq!(spec.cargo_ledger_name, spec.name);
        }

        let spec = Device::NanoSPlus.spec();
        assert_eq!(spec.name, "nanosp");
        assert_eq!(spec.build_dir_name, "nanos2");
        assert_eq!(spec.cargo_ledger_name, "nanosplus");
        assert_eq!(Device::NanoSPlus.sdk_model(Language::C), "nanos2");
        assert_eq!(Device::NanoSPlus.sdk_model(Language::Rust), "nanosplus");
    }

    #[test]
    fn sdk_env_vars_follow_the_container_convention() {
        let vars = Device::ALL.map(Device::sdk_env_var);

        assert_eq!(
            vars,
            [
                "NANOS_SDK",
                "NANOSP_SDK",
                "NANOX_SDK",
                "STAX_SDK",
                "FLEX_SDK",
                "APEX_P_SDK",
                "APEX_M_SDK",
            ]
        );
    }

    #[test]
    fn target_ids_match_the_values_the_extension_used_so_far() {
        let ids = Device::ALL.map(|device| device.spec().target_id);

        assert_eq!(
            ids,
            [
                0x3110_0004,
                0x3310_0004,
                0x3300_0004,
                0x3320_0004,
                0x3330_0004,
                0x3340_0004,
                0x3350_0004,
            ]
        );
    }

    #[test]
    fn serializes_as_the_canonical_name_and_nothing_else() {
        for device in Device::ALL {
            let json = serde_json::to_string(&device).unwrap();
            assert_eq!(json, format!("\"{}\"", device.spec().name));
            assert_eq!(serde_json::from_str::<Device>(&json).unwrap(), device);
        }
        assert!(serde_json::from_str::<Device>("\"nanos+\"").is_err());
    }
}
