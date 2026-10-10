use ledger_core::manifest::{
    self, AppManifest, LegacyManifest, ParsedManifest, TestDependency, TestSuite, UseCaseOrigin,
};
use ledger_core::{Device, Error, Language};

const BOILERPLATE: &str = include_str!("fixtures/boilerplate_c.toml");

fn current(source: &str) -> AppManifest {
    match manifest::parse(source).expect("manifest should parse") {
        ParsedManifest::Current(manifest) => manifest,
        ParsedManifest::Legacy(_) => panic!("expected a current manifest"),
    }
}

fn rejected(source: &str) -> Error {
    manifest::parse(source).expect_err("manifest should be rejected")
}

/// A valid `[app]` section followed by `extra`.
fn app(extra: &str) -> String {
    format!("[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = [\"nanox\"]\n{extra}")
}

fn use_cases(manifest: &AppManifest) -> Vec<(&str, UseCaseOrigin)> {
    manifest
        .build_use_cases
        .iter()
        .map(|use_case| (use_case.name.as_str(), use_case.origin))
        .collect()
}

fn suite<'a>(manifest: &'a AppManifest, name: &str) -> &'a TestSuite {
    manifest.test_suite(name).expect("suite should exist")
}

fn suite_names(manifest: &AppManifest) -> Vec<&str> {
    manifest
        .test_suites
        .iter()
        .map(|suite| suite.name.as_str())
        .collect()
}

fn use_case_names(suite: &TestSuite) -> Vec<&str> {
    suite
        .use_cases
        .iter()
        .map(|use_case| use_case.name.as_str())
        .collect()
}

fn dependency(url: &str, git_ref: &str, use_case: &str) -> TestDependency {
    TestDependency {
        git_repo_url: url.to_owned(),
        git_repo_ref: git_ref.to_owned(),
        use_case: use_case.to_owned(),
    }
}

#[test]
fn parses_the_boilerplate_manifest() {
    let manifest = current(BOILERPLATE);

    assert_eq!(manifest.language, Language::C);
    assert_eq!(manifest.build_directory, "./");
    assert_eq!(
        manifest.devices,
        [
            Device::NanoX,
            Device::NanoSPlus,
            Device::Stax,
            Device::Flex,
            Device::ApexP
        ]
    );
    assert_eq!(
        use_cases(&manifest),
        [
            ("release", UseCaseOrigin::BuiltIn),
            ("debug", UseCaseOrigin::Manifest)
        ]
    );
}

#[test]
fn parses_every_test_suite_with_its_dependencies() {
    let manifest = current(BOILERPLATE);

    let suites: Vec<_> = manifest
        .test_suites
        .iter()
        .map(|suite| (suite.name.as_str(), suite.directory.as_deref()))
        .collect();
    assert_eq!(
        suites,
        [
            ("standalone", Some("./tests/standalone/")),
            ("swap", Some("./tests/swap/"))
        ]
    );

    let swap = suite(&manifest, "swap");
    assert_eq!(use_case_names(swap), ["testing_with_latest"]);
    assert_eq!(
        swap.use_cases[0].dependencies,
        [
            dependency(
                "https://github.com/LedgerHQ/app-exchange",
                "develop",
                "dbg_use_test_keys"
            ),
            dependency(
                "https://github.com/LedgerHQ/app-ethereum",
                "develop",
                "use_test_keys"
            ),
        ]
    );
    assert!(suite(&manifest, "standalone").use_cases.is_empty());
}

#[test]
fn debug_defined_by_the_manifest_is_not_reported_as_built_in() {
    // The extension must tell `make debug` (manifest) from its own `DEBUG=1` default.
    let manifest = current(&app("[use_cases]\ndebug = \"DEBUG=1\"\n"));

    assert_eq!(
        use_cases(&manifest),
        [
            ("release", UseCaseOrigin::BuiltIn),
            ("debug", UseCaseOrigin::Manifest)
        ]
    );
}

#[test]
fn release_and_debug_are_added_when_the_manifest_has_no_use_cases() {
    let manifest = current(&app(""));

    assert_eq!(
        use_cases(&manifest),
        [
            ("release", UseCaseOrigin::BuiltIn),
            ("debug", UseCaseOrigin::BuiltIn)
        ]
    );
}

#[test]
fn manifest_use_cases_keep_their_file_order_after_the_built_in_ones() {
    let manifest = current(&app("[use_cases]\nzeta = \"Z=1\"\nalpha = \"A=1\"\n"));

    assert_eq!(
        use_cases(&manifest),
        [
            ("release", UseCaseOrigin::BuiltIn),
            ("debug", UseCaseOrigin::BuiltIn),
            ("zeta", UseCaseOrigin::Manifest),
            ("alpha", UseCaseOrigin::Manifest),
        ]
    );
}

#[test]
fn release_defined_by_the_manifest_is_not_duplicated() {
    let manifest = current(&app("[use_cases]\nrelease = \"OPT=2\"\n"));

    assert_eq!(
        use_cases(&manifest),
        [
            ("release", UseCaseOrigin::Manifest),
            ("debug", UseCaseOrigin::BuiltIn)
        ]
    );
}

#[test]
fn devices_can_be_a_comma_separated_string() {
    let source = "[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = \"nanos+, stax\"\n";

    assert_eq!(current(source).devices, [Device::NanoSPlus, Device::Stax]);
}

#[test]
fn devices_can_use_any_name_the_tooling_knows() {
    let source = "[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = [\"nanosp\", \"nanosplus\", \"NANOS2\"]\n";

    assert_eq!(current(source).devices, [Device::NanoSPlus; 3]);
}

#[test]
fn sdk_is_case_insensitive() {
    let source = "[app]\nsdk = \"Rust\"\nbuild_directory = \"./\"\ndevices = [\"flex\"]\n";

    assert_eq!(current(source).language, Language::Rust);
}

#[test]
fn mandatory_app_properties_are_reported_in_order() {
    assert!(matches!(rejected(""), Error::MissingProperty("app.sdk")));
    assert!(matches!(
        rejected("[app]\nsdk = \"C\"\n"),
        Error::MissingProperty("app.build_directory")
    ));
    assert!(matches!(
        rejected("[app]\nsdk = \"C\"\nbuild_directory = \"./\"\n"),
        Error::MissingProperty("app.devices")
    ));
}

#[test]
fn unknown_language_is_rejected() {
    let source = "[app]\nsdk = \"go\"\nbuild_directory = \"./\"\ndevices = [\"flex\"]\n";

    assert!(matches!(rejected(source), Error::InvalidLanguage(language) if language == "go"));
}

#[test]
fn unknown_device_is_rejected() {
    let source =
        "[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = [\"nanox\", \"nanosx\"]\n";

    assert!(matches!(rejected(source), Error::InvalidDevice(device) if device == "nanosx"));
}

#[test]
fn empty_device_list_is_rejected() {
    let source = "[app]\nsdk = \"C\"\nbuild_directory = \"./\"\ndevices = []\n";

    assert!(matches!(rejected(source), Error::NoDevices));
}

#[test]
fn malformed_toml_is_rejected() {
    assert!(matches!(rejected("[app\nsdk = "), Error::Toml(_)));
}

#[test]
fn rust_app_table_marks_the_legacy_format() {
    let parsed = manifest::parse("[rust-app]\nmanifest-path = \"./app/Cargo.toml\"\n").unwrap();

    assert_eq!(
        parsed,
        ParsedManifest::Legacy(LegacyManifest {
            cargo_manifest_path: "./app/Cargo.toml".to_owned()
        })
    );
    assert_eq!(LegacyManifest::LANGUAGE, Language::Rust);
    assert_eq!(
        LegacyManifest::DEVICES,
        [Device::NanoS, Device::NanoSPlus, Device::NanoX]
    );
}

#[test]
fn legacy_manifest_requires_the_cargo_path() {
    assert!(matches!(
        rejected("[rust-app]\n"),
        Error::MissingProperty("rust-app.manifest-path")
    ));
}

#[test]
fn a_manifest_without_tests_has_no_test_suite() {
    assert!(current(&app("")).test_suites.is_empty());
}

#[test]
fn standalone_directory_wins_over_the_legacy_one() {
    let manifest = current(&app(
        "[pytest.standalone]\ndirectory = \"./new/\"\n[tests]\npytest_directory = \"./old/\"\n",
    ));

    assert_eq!(suite_names(&manifest), ["standalone"]);
    assert_eq!(
        suite(&manifest, "standalone").directory.as_deref(),
        Some("./new/")
    );
}

#[test]
fn the_legacy_tests_table_becomes_the_standalone_suite() {
    let manifest = current(&app(concat!(
        "[tests]\n",
        "pytest_directory = \"./old/\"\n",
        "[tests.dependencies]\n",
        "default = [\n",
        "  { url = \"https://example.com/app-x\", ref = \"main\", use_case = \"debug\" },\n",
        "]\n",
    )));

    assert_eq!(suite_names(&manifest), ["standalone"]);
    let standalone = suite(&manifest, "standalone");
    assert_eq!(standalone.directory.as_deref(), Some("./old/"));
    assert_eq!(use_case_names(standalone), ["default"]);
    assert_eq!(
        standalone.use_cases[0].dependencies,
        [dependency("https://example.com/app-x", "main", "debug")]
    );
}

#[test]
fn the_legacy_standalone_suite_comes_before_the_other_suites() {
    let manifest = current(&app(
        "[pytest.swap]\ndirectory = \"./swap/\"\n[tests]\npytest_directory = \"./old/\"\n",
    ));

    assert_eq!(suite_names(&manifest), ["standalone", "swap"]);
}

#[test]
fn empty_directories_are_ignored() {
    let both = current(&app(
        "[pytest.standalone]\ndirectory = \"\"\n[tests]\npytest_directory = \"./old/\"\n",
    ));
    let only_empty = current(&app("[pytest.standalone]\ndirectory = \"\"\n"));

    assert_eq!(
        suite(&both, "standalone").directory.as_deref(),
        Some("./old/")
    );
    assert_eq!(suite(&only_empty, "standalone").directory, None);
}

#[test]
fn current_and_legacy_use_cases_are_merged_and_the_current_one_wins() {
    let manifest = current(&app(concat!(
        "[pytest.standalone.dependencies]\n",
        "a = [{ url = \"https://example.com/current\", ref = \"main\", use_case = \"debug\" }]\n",
        "[tests.dependencies]\n",
        "a = [{ url = \"https://example.com/legacy\", ref = \"main\", use_case = \"debug\" }]\n",
        "b = [{ url = \"https://example.com/other\", ref = \"main\", use_case = \"debug\" }]\n",
    )));

    let standalone = suite(&manifest, "standalone");
    assert_eq!(use_case_names(standalone), ["a", "b"]);
    assert_eq!(
        standalone.use_cases[0].dependencies,
        [dependency("https://example.com/current", "main", "debug")]
    );
}

#[test]
fn test_dependency_without_ref_is_rejected() {
    let source = app(concat!(
        "[tests.dependencies]\n",
        "default = [{ url = \"https://example.com/app-x\", use_case = \"debug\" }]\n",
    ));

    assert!(matches!(rejected(&source), Error::MissingProperty("ref")));
}
