//! What the user chose for one app, and how that is saved between sessions.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::device::Device;
use crate::enforcer;
use crate::language::Language;
use crate::manifest::AppManifest;
use crate::target::{Resolution, Target};
use crate::test_tree::TestSelection;

/// The choices for one app. Each one is always something the app offers: the fields are private
/// and every change is checked against the [`App`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSelection {
    target: Target,
    build_use_case: Option<String>,
    variant: Option<String>,
    test_suite: Option<String>,
    /// One check of the guideline enforcer. `None` runs all of them. It is not saved.
    enforcer_check: Option<String>,
    /// Tests of the selected suite. `None`: none was chosen, which means all of them.
    pub tests: Option<TestSelection>,
}

impl AppSelection {
    /// Restores what was saved, and chooses the defaults for the rest. `None` when the app
    /// supports no device.
    pub fn new(app: &App, saved: &SavedSelection, default_device: Option<Device>) -> Option<Self> {
        let wanted = saved
            .target
            .as_ref()
            .and_then(|target| target.to_target(&app.devices))
            .or_else(|| default_device.map(Target::one));
        let target = match wanted {
            Some(wanted) => wanted.resolve(&app.devices)?.target,
            None => Target::one(*app.devices.first()?),
        };
        let mut selection = Self {
            target,
            build_use_case: saved.build_use_case.clone(),
            variant: saved.variant.clone(),
            test_suite: saved.test_suite.clone(),
            enforcer_check: None,
            tests: saved
                .tests
                .as_ref()
                .map(|ids| TestSelection::from_ids(ids.iter().cloned())),
        };
        selection.reconcile(app);
        Some(selection)
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn build_use_case(&self) -> Option<&str> {
        self.build_use_case.as_deref()
    }

    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    pub fn test_suite(&self) -> Option<&str> {
        self.test_suite.as_deref()
    }

    pub fn enforcer_check(&self) -> Option<&str> {
        self.enforcer_check.as_deref()
    }

    /// `All` or the name of a check. Returns whether the enforcer has this check.
    pub fn set_enforcer_check(&mut self, check: &str) -> bool {
        if check == enforcer::ALL {
            self.enforcer_check = None;
            return true;
        }
        let known = enforcer::CHECKS.contains(&check);
        if known {
            self.enforcer_check = Some(check.to_owned());
        }
        known
    }

    /// Keeps the devices of `wanted` that the app supports. `None`, and nothing changes, when
    /// the app supports no device.
    pub fn set_target(&mut self, wanted: &Target, app: &App) -> Option<Resolution> {
        let resolution = wanted.resolve(&app.devices)?;
        self.target = resolution.target.clone();
        Some(resolution)
    }

    /// Returns whether the app has this build use case.
    pub fn set_build_use_case(&mut self, name: &str, app: &App) -> bool {
        let known = app
            .build_use_cases
            .iter()
            .any(|use_case| use_case.name == name);
        if known {
            self.build_use_case = Some(name.to_owned());
        }
        known
    }

    /// Returns whether the app has this variant.
    pub fn set_variant(&mut self, value: &str, app: &App) -> bool {
        let known =
            variants_of(app).is_some_and(|values| values.iter().any(|known| known == value));
        if known {
            self.variant = Some(value.to_owned());
        }
        known
    }

    /// Returns whether the app has this test suite. The tests chosen in another suite are
    /// forgotten.
    pub fn set_test_suite(&mut self, name: &str, app: &App) -> bool {
        let known = app.test_suites.iter().any(|suite| suite.name == name);
        if known && self.test_suite.as_deref() != Some(name) {
            self.test_suite = Some(name.to_owned());
            self.tests = None;
        }
        known
    }

    /// Brings the choices back into what the app offers, after the app was detected again.
    /// What no longer exists gives way to the default.
    pub fn reconcile(&mut self, app: &App) {
        if let Some(resolution) = self.target.resolve(&app.devices) {
            self.target = resolution.target;
        }
        self.build_use_case = self
            .build_use_case
            .take()
            .filter(|name| {
                app.build_use_cases
                    .iter()
                    .any(|use_case| &use_case.name == name)
            })
            .or_else(|| {
                app.build_use_cases
                    .first()
                    .map(|use_case| use_case.name.clone())
            });
        self.reconcile_test_suite(app);
        self.reconcile_variant(app);
    }

    fn reconcile_test_suite(&mut self, app: &App) {
        let standalone = app
            .test_suites
            .iter()
            .find(|suite| suite.name == AppManifest::STANDALONE_SUITE);
        let suite = self
            .test_suite
            .clone()
            .filter(|name| app.test_suites.iter().any(|suite| &suite.name == name))
            .or_else(|| {
                standalone
                    .or(app.test_suites.first())
                    .map(|suite| suite.name.clone())
            });
        if suite != self.test_suite {
            self.tests = None;
        }
        self.test_suite = suite;
    }

    /// The variants are only known once the Makefile of a C app was asked. Until then, what was
    /// saved is kept.
    fn reconcile_variant(&mut self, app: &App) {
        if app.language != Language::C {
            self.variant = None;
            return;
        }
        let Some(makefile) = &app.makefile else {
            return;
        };
        self.variant = makefile.variants.as_ref().map(|variants| {
            self.variant
                .take()
                .filter(|value| variants.values.contains(value))
                .unwrap_or_else(|| variants.default_value(&makefile.name).to_owned())
        });
    }

    /// What to save of the choices. A target of every device is saved as `All`, so it follows
    /// the app when it supports more devices later.
    pub fn saved(&self, app: &App) -> SavedSelection {
        let target = if self.target.is_all(&app.devices) {
            SavedTarget::All
        } else {
            SavedTarget::Devices(self.target.devices().clone())
        };
        SavedSelection {
            target: Some(target),
            build_use_case: self.build_use_case.clone(),
            variant: self.variant.clone(),
            test_suite: self.test_suite.clone(),
            tests: self
                .tests
                .as_ref()
                .map(|tests| tests.ids().map(str::to_owned).collect()),
        }
    }
}

fn variants_of(app: &App) -> Option<&Vec<String>> {
    let variants = app.makefile.as_ref()?.variants.as_ref()?;
    Some(&variants.values)
}

/// The choices for one app as they are stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<SavedTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_use_case: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_suite: Option<String>,
    /// Ids of the selected tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedTarget {
    /// Every device the app supports, whichever they are.
    All,
    Devices(BTreeSet<Device>),
}

impl SavedTarget {
    fn to_target(&self, compatible: &[Device]) -> Option<Target> {
        match self {
            Self::All => Target::all(compatible),
            Self::Devices(devices) => Target::new(devices.iter().copied()),
        }
    }

    /// Reads the former `selectedDevice` setting: `All` or the display name of a device.
    fn from_legacy(value: &str) -> Option<Self> {
        if value == Target::ALL_LABEL {
            return Some(Self::All);
        }
        let device = Device::from_display_name(value)?;
        Some(Self::Devices(BTreeSet::from([device])))
    }
}

impl SavedSelection {
    /// Reads the former `appSettings` of an app folder. The selected tests are not taken over:
    /// they were saved in another form.
    pub fn from_legacy_settings(settings: &serde_json::Value) -> Self {
        let text = |key: &str| {
            let value = settings.get(key)?.as_str()?;
            (!value.is_empty()).then(|| value.to_owned())
        };
        Self {
            target: text("selectedDevice").and_then(|value| SavedTarget::from_legacy(&value)),
            build_use_case: text("selectedUseCase"),
            variant: text("selectedVariant"),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::c_app::{MakefileInfo, Variants};
    use crate::manifest::{self, ParsedManifest};
    use crate::test_tree::TestTree;

    const SUITES: &str = "
[pytest.swap]
directory = \"./tests/swap/\"
[pytest.standalone]
directory = \"./tests/standalone/\"
";

    fn app_with(language: &str, devices: &str, extra: &str) -> App {
        let source = format!(
            "[app]\nsdk = \"{language}\"\nbuild_directory = \"./\"\ndevices = [{devices}]\n{extra}"
        );
        let ParsedManifest::Current(manifest) = manifest::parse(&source).unwrap() else {
            panic!("a current manifest");
        };
        let rust = (language == "rust").then(|| crate::rust_app::RustAppPackage {
            package_name: "x".to_owned(),
            app_name: "X".to_owned(),
        });
        App::from_manifest("/work/app-x", &manifest, rust)
    }

    fn app(devices: &str) -> App {
        app_with("c", devices, SUITES)
    }

    fn with_variants(mut app: App, name: &str, values: &[&str]) -> App {
        app.makefile = Some(MakefileInfo {
            name: name.to_owned(),
            variants: Some(Variants {
                name: "COIN".to_owned(),
                values: values.iter().map(|value| (*value).to_owned()).collect(),
            }),
        });
        app
    }

    fn selection(app: &App) -> AppSelection {
        AppSelection::new(app, &SavedSelection::default(), None).unwrap()
    }

    fn target(devices: &[Device]) -> Target {
        Target::new(devices.iter().copied()).unwrap()
    }

    #[test]
    fn without_a_saved_choice_the_first_device_and_the_defaults_are_taken() {
        let selection = selection(&app("\"stax\", \"flex\""));

        assert_eq!(selection.target(), &Target::one(Device::Stax));
        assert_eq!(selection.build_use_case(), Some("release"));
        assert_eq!(selection.test_suite(), Some("standalone"));
        assert_eq!(selection.variant(), None);
        assert_eq!(selection.tests, None);
    }

    #[test]
    fn the_default_device_is_used_when_the_app_supports_it() {
        let app = app("\"stax\", \"flex\"");

        let chosen =
            AppSelection::new(&app, &SavedSelection::default(), Some(Device::Flex)).unwrap();
        let unsupported =
            AppSelection::new(&app, &SavedSelection::default(), Some(Device::NanoX)).unwrap();

        assert_eq!(chosen.target(), &Target::one(Device::Flex));
        assert_eq!(unsupported.target(), &Target::one(Device::Stax));
    }

    #[test]
    fn an_app_that_supports_no_device_has_no_selection() {
        let mut app = app("\"stax\"");
        app.devices.clear();

        assert_eq!(
            AppSelection::new(&app, &SavedSelection::default(), None),
            None
        );
    }

    #[test]
    fn the_test_suite_defaults_to_standalone_then_to_the_first() {
        let mut app = app("\"stax\"");
        assert_eq!(selection(&app).test_suite(), Some("standalone"));

        app.test_suites.retain(|suite| suite.name != "standalone");
        assert_eq!(selection(&app).test_suite(), Some("swap"));

        app.test_suites.clear();
        assert_eq!(selection(&app).test_suite(), None);
    }

    #[test]
    fn a_target_can_only_be_set_within_the_devices_of_the_app() {
        let app = app("\"stax\", \"flex\"");
        let mut selection = selection(&app);

        let resolution = selection
            .set_target(&target(&[Device::Flex, Device::NanoX]), &app)
            .unwrap();

        assert_eq!(selection.target(), &Target::one(Device::Flex));
        assert_eq!(resolution.unsupported, BTreeSet::from([Device::NanoX]));
    }

    #[test]
    fn a_target_the_app_does_not_support_at_all_falls_back_to_its_first_device() {
        let app = app("\"stax\", \"flex\"");
        let mut selection = selection(&app);
        selection.set_target(&Target::one(Device::Flex), &app);

        selection.set_target(&Target::one(Device::NanoX), &app);

        assert_eq!(selection.target(), &Target::one(Device::Stax));
    }

    #[test]
    fn only_a_use_case_the_app_has_can_be_chosen() {
        let app = app_with(
            "c",
            "\"stax\"",
            "[use_cases]\ndebug = \"DEBUG=1\"\nfast = \"FAST=1\"\n",
        );
        let mut selection = selection(&app);

        assert!(selection.set_build_use_case("fast", &app));
        assert!(!selection.set_build_use_case("nope", &app));

        assert_eq!(selection.build_use_case(), Some("fast"));
    }

    #[test]
    fn only_a_variant_the_app_has_can_be_chosen() {
        let app = with_variants(app("\"stax\""), "Boilerplate", &["BTC", "LTC"]);
        let mut selection = selection(&app);

        assert!(selection.set_variant("LTC", &app));
        assert!(!selection.set_variant("DOGE", &app));

        assert_eq!(selection.variant(), Some("LTC"));
    }

    #[test]
    fn the_variant_defaults_to_the_one_named_like_the_app() {
        let app = with_variants(app("\"stax\""), "ltc", &["BTC", "LTC"]);

        assert_eq!(selection(&app).variant(), Some("LTC"));
    }

    #[test]
    fn a_saved_variant_is_kept_until_the_makefile_has_been_asked() {
        let saved = SavedSelection {
            variant: Some("LTC".to_owned()),
            ..SavedSelection::default()
        };
        let before = app("\"stax\"");
        let mut selection = AppSelection::new(&before, &saved, None).unwrap();
        assert_eq!(selection.variant(), Some("LTC"));

        let after = with_variants(before, "Boilerplate", &["BTC", "LTC"]);
        selection.reconcile(&after);
        assert_eq!(selection.variant(), Some("LTC"));

        let changed = with_variants(after, "Boilerplate", &["BTC", "DOGE"]);
        selection.reconcile(&changed);
        assert_eq!(selection.variant(), Some("BTC"));
    }

    #[test]
    fn a_rust_app_has_no_variant() {
        let saved = SavedSelection {
            variant: Some("LTC".to_owned()),
            ..SavedSelection::default()
        };
        let app = app_with("rust", "\"stax\"", "");

        let selection = AppSelection::new(&app, &saved, None).unwrap();

        assert_eq!(selection.variant(), None);
    }

    #[test]
    fn reconciling_drops_what_the_app_no_longer_offers() {
        let before = app("\"stax\", \"flex\"");
        let mut selection = selection(&before);
        selection.set_target(&Target::one(Device::Flex), &before);
        selection.set_test_suite("swap", &before);
        let after = app_with("c", "\"stax\"", "[pytest.other]\ndirectory = \"./t/\"\n");

        selection.reconcile(&after);

        assert_eq!(selection.target(), &Target::one(Device::Stax));
        assert_eq!(selection.test_suite(), Some("other"));
    }

    #[test]
    fn reconciling_keeps_what_the_app_still_offers() {
        let app = app("\"stax\", \"flex\"");
        let mut selection = selection(&app);
        selection.set_target(&Target::one(Device::Flex), &app);
        selection.set_test_suite("swap", &app);
        let before = selection.clone();

        selection.reconcile(&app.clone());

        assert_eq!(selection, before);
    }

    #[test]
    fn the_enforcer_runs_every_check_unless_one_is_chosen() {
        let mut selection = selection(&app("\"stax\""));
        assert_eq!(selection.enforcer_check(), None);

        assert!(selection.set_enforcer_check("icons"));
        assert_eq!(selection.enforcer_check(), Some("icons"));
        assert!(!selection.set_enforcer_check("nope"));
        assert_eq!(selection.enforcer_check(), Some("icons"));

        assert!(selection.set_enforcer_check("All"));
        assert_eq!(selection.enforcer_check(), None);
    }

    #[test]
    fn changing_the_suite_forgets_the_tests_of_the_other_one() {
        let app = app("\"stax\"");
        let mut selection = selection(&app);
        let tree = TestTree::from_collect_output("test_a.py::test_x\n");
        selection.tests = Some(TestSelection::all(&tree));

        assert!(selection.set_test_suite("standalone", &app));
        assert!(selection.tests.is_some());

        assert!(selection.set_test_suite("swap", &app));
        assert_eq!(selection.tests, None);
    }

    #[test]
    fn every_device_is_saved_as_all_and_follows_the_app() {
        let before = app("\"stax\", \"flex\"");
        let mut chosen = selection(&before);
        chosen.set_target(&Target::all(&before.devices).unwrap(), &before);
        let saved = chosen.saved(&before);
        assert_eq!(saved.target, Some(SavedTarget::All));

        let after = app("\"stax\", \"flex\", \"nanox\"");
        let restored = AppSelection::new(&after, &saved, None).unwrap();

        assert_eq!(restored.target(), &Target::all(&after.devices).unwrap());
    }

    #[test]
    fn some_devices_are_saved_as_they_are() {
        let app = app("\"stax\", \"flex\", \"nanox\"");
        let mut chosen = selection(&app);
        chosen.set_target(&target(&[Device::Stax, Device::NanoX]), &app);

        let saved = chosen.saved(&app);

        assert_eq!(
            saved.target,
            Some(SavedTarget::Devices(BTreeSet::from([
                Device::NanoX,
                Device::Stax
            ])))
        );
    }

    #[test]
    fn the_only_device_of_an_app_is_not_saved_as_all() {
        let app = app("\"stax\"");

        assert_eq!(
            selection(&app).saved(&app).target,
            Some(SavedTarget::Devices(BTreeSet::from([Device::Stax])))
        );
    }

    #[test]
    fn what_is_saved_is_restored() {
        let app = with_variants(app("\"stax\", \"flex\""), "Boilerplate", &["BTC", "LTC"]);
        let mut chosen = selection(&app);
        chosen.set_target(&Target::one(Device::Flex), &app);
        chosen.set_variant("LTC", &app);
        chosen.set_test_suite("swap", &app);
        let tree = TestTree::from_collect_output("test_a.py::test_x\ntest_a.py::test_y\n");
        chosen.tests = Some(TestSelection::all(&tree));

        let saved = chosen.saved(&app);
        let json = serde_json::to_string(&saved).unwrap();
        let restored: SavedSelection = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, saved);
        assert_eq!(AppSelection::new(&app, &restored, None), Some(chosen));
    }

    #[test]
    fn a_saved_choice_the_app_no_longer_has_gives_way_to_the_default() {
        let app = app("\"stax\"");
        let saved = SavedSelection {
            target: Some(SavedTarget::Devices(BTreeSet::from([Device::NanoX]))),
            build_use_case: Some("gone".to_owned()),
            test_suite: Some("gone".to_owned()),
            ..SavedSelection::default()
        };

        let selection = AppSelection::new(&app, &saved, None).unwrap();

        assert_eq!(selection.target(), &Target::one(Device::Stax));
        assert_eq!(selection.build_use_case(), Some("release"));
        assert_eq!(selection.test_suite(), Some("standalone"));
    }

    #[test]
    fn nothing_is_saved_for_a_choice_that_was_not_made() {
        let json = serde_json::to_string(&SavedSelection::default()).unwrap();

        assert_eq!(json, "{}");
        assert_eq!(
            serde_json::from_str::<SavedSelection>("{}").unwrap(),
            SavedSelection::default()
        );
    }

    #[test]
    fn the_former_settings_are_read() {
        let settings = json!({
            "selectedDevice": "Nano S Plus",
            "selectedUseCase": "debug",
            "selectedVariant": "LTC",
            "selectedTests": ["tests/a.py"],
        });

        assert_eq!(
            SavedSelection::from_legacy_settings(&settings),
            SavedSelection {
                target: Some(SavedTarget::Devices(BTreeSet::from([Device::NanoSPlus]))),
                build_use_case: Some("debug".to_owned()),
                variant: Some("LTC".to_owned()),
                ..SavedSelection::default()
            }
        );
    }

    #[test]
    fn the_former_all_setting_stays_all() {
        let settings = json!({ "selectedDevice": "All" });

        assert_eq!(
            SavedSelection::from_legacy_settings(&settings).target,
            Some(SavedTarget::All)
        );
    }

    #[test]
    fn unusable_former_settings_are_ignored() {
        let settings =
            json!({ "selectedDevice": "Nokia 3310", "selectedUseCase": "", "selectedVariant": 3 });

        assert_eq!(
            SavedSelection::from_legacy_settings(&settings),
            SavedSelection::default()
        );
        assert_eq!(
            SavedSelection::from_legacy_settings(&json!(null)),
            SavedSelection::default()
        );
    }
}
