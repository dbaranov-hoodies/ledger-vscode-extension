use std::path::Path;

use ledger_core::detect::{self, Detected, FolderFiles, ManifestWarning};
use ledger_core::{Device, Error, Language};

const BOILERPLATE: &str = include_str!("fixtures/boilerplate_c.toml");
const C_MAKEFILE: &str = "APPNAME = X\ninclude $(BOLOS_SDK)/Makefile.defines\n";
const CARGO_TOML: &str =
    "[package]\nname = \"app-x-rust\"\n[package.metadata.ledger]\nname = \"X\"\n";
const RUST_MANIFEST: &str =
    "[app]\nsdk = \"rust\"\nbuild_directory = \"./app\"\ndevices = [\"stax\", \"flex\"]\n";
const LEGACY_MANIFEST: &str = "[rust-app]\nmanifest-path = \"./app/Cargo.toml\"\n";
const FOLDER: &str = "/work/app-x";

fn files(entries: &[(&str, &str)]) -> FolderFiles {
    FolderFiles::new(
        entries
            .iter()
            .map(|(path, content)| ((*path).to_owned(), (*content).to_owned())),
    )
}

fn detect(entries: &[(&str, &str)]) -> Option<Detected> {
    detect::detect_app(Path::new(FOLDER), &files(entries)).expect("detection should succeed")
}

#[test]
fn a_folder_without_the_files_holds_no_app() {
    assert_eq!(detect(&[]), None);
    assert_eq!(detect(&[("README.md", "")]), None);
}

#[test]
fn a_current_c_manifest_gives_the_app_without_a_warning() {
    let detected = detect(&[("ledger_app.toml", BOILERPLATE)]).unwrap();

    assert_eq!(detected.warning, None);
    let app = detected.app;
    assert_eq!(app.folder, Path::new(FOLDER));
    assert_eq!(app.language, Language::C);
    assert_eq!(app.build_directory, "./");
    assert_eq!(app.devices.len(), 5);
    assert_eq!(app.rust, None);
    assert_eq!(app.makefile, None);
    assert_eq!(app.build_use_cases.len(), 2);
    assert_eq!(app.test_suites.len(), 2);
}

#[test]
fn a_rust_manifest_reads_the_cargo_toml_of_the_build_directory() {
    let detected = detect(&[
        ("ledger_app.toml", RUST_MANIFEST),
        ("app/Cargo.toml", CARGO_TOML),
        ("Cargo.toml", "this one is not read"),
    ])
    .unwrap();

    let rust = detected.app.rust.as_ref().unwrap();
    assert_eq!(rust.package_name, "app-x-rust");
    assert_eq!(rust.app_name, "X");
    assert_eq!(detected.app.devices, [Device::Stax, Device::Flex]);
    assert_eq!(detected.app.name(), Some("X"));
}

#[test]
fn a_rust_app_without_its_cargo_toml_is_an_error() {
    let error = detect::detect_app(
        Path::new(FOLDER),
        &files(&[("ledger_app.toml", RUST_MANIFEST)]),
    )
    .unwrap_err();

    assert!(matches!(error, Error::MissingFile(path) if path == "./app/Cargo.toml"));
}

#[test]
fn a_broken_manifest_is_an_error_not_a_missing_app() {
    let error = detect::detect_app(
        Path::new(FOLDER),
        &files(&[("ledger_app.toml", "[app]\nsdk = \"cobol\"\n")]),
    )
    .unwrap_err();

    assert!(matches!(error, Error::InvalidLanguage(_)));
}

#[test]
fn a_legacy_manifest_gives_a_rust_app_for_the_three_nano_models() {
    let detected = detect(&[
        ("ledger_app.toml", LEGACY_MANIFEST),
        ("app/Cargo.toml", CARGO_TOML),
    ])
    .unwrap();

    assert_eq!(detected.warning, Some(ManifestWarning::Deprecated));
    let app = detected.app;
    assert_eq!(app.language, Language::Rust);
    assert_eq!(app.build_directory, "./app");
    assert_eq!(
        app.devices,
        [Device::NanoS, Device::NanoSPlus, Device::NanoX]
    );
    assert!(app.build_use_cases.is_empty());
    assert_eq!(app.rust.unwrap().app_name, "X");
}

#[test]
fn a_makefile_that_includes_the_sdk_gives_a_c_app_with_the_default_devices() {
    let detected = detect(&[("Makefile", C_MAKEFILE)]).unwrap();

    assert_eq!(detected.warning, Some(ManifestWarning::Missing));
    let app = detected.app;
    assert_eq!(app.language, Language::C);
    assert_eq!(app.build_directory, "./");
    assert_eq!(
        app.devices,
        [
            Device::NanoSPlus,
            Device::NanoX,
            Device::Stax,
            Device::Flex,
            Device::ApexP
        ]
    );
    assert_eq!(app.name(), None);
}

#[test]
fn the_build_directory_of_a_makefile_app_is_where_its_makefile_is() {
    let detected = detect(&[("app/Makefile", C_MAKEFILE)]).unwrap();

    assert_eq!(detected.app.build_directory, "./app");
}

#[test]
fn a_makefile_that_does_not_include_the_sdk_is_not_an_app() {
    assert_eq!(detect(&[("Makefile", "all:\n\techo hi\n")]), None);
}

#[test]
fn the_first_makefile_that_includes_the_sdk_is_the_one() {
    let detected = detect(&[
        ("Makefile", "all:\n"),
        ("docs/Makefile", "docs:\n"),
        ("app/Makefile", C_MAKEFILE),
    ])
    .unwrap();

    assert_eq!(detected.app.build_directory, "./app");
}

#[test]
fn a_manifest_wins_over_a_makefile_and_the_closest_one_over_a_deeper_one() {
    let detected = detect(&[
        ("Makefile", C_MAKEFILE),
        ("sub/ledger_app.toml", RUST_MANIFEST),
        ("ledger_app.toml", BOILERPLATE),
    ])
    .unwrap();

    assert_eq!(detected.app.language, Language::C);
    assert_eq!(detected.warning, None);
}

#[test]
fn an_app_without_a_current_manifest_has_its_tests_where_conftest_py_is() {
    let detected = detect(&[("Makefile", C_MAKEFILE), ("tests/conftest.py", "")]).unwrap();

    let suite = &detected.app.test_suites[0];
    assert_eq!(suite.name, "standalone");
    assert_eq!(suite.directory.as_deref(), Some("./tests"));
}

#[test]
fn conftest_py_at_the_top_of_the_app_makes_the_app_folder_the_tests_directory() {
    let detected = detect(&[("Makefile", C_MAKEFILE), ("conftest.py", "")]).unwrap();

    assert_eq!(detected.app.test_suites[0].directory.as_deref(), Some("./"));
}

#[test]
fn an_app_without_conftest_py_has_no_tests() {
    let detected = detect(&[("Makefile", C_MAKEFILE)]).unwrap();

    assert!(detected.app.test_suites.is_empty());
}

#[test]
fn paths_may_start_with_dot_slash_and_use_backslashes() {
    let detected = detect(&[
        ("./ledger_app.toml", RUST_MANIFEST),
        (".\\app\\Cargo.toml", CARGO_TOML),
    ])
    .unwrap();

    assert_eq!(detected.app.name(), Some("X"));
}

#[test]
fn the_compose_file_is_the_closest_one() {
    let detected = detect(&[
        ("Makefile", C_MAKEFILE),
        ("docker/docker-compose.yml", ""),
        ("deep/er/docker-compose.yml", ""),
    ])
    .unwrap();

    assert_eq!(
        detected.app.compose_file.as_deref(),
        Some("docker/docker-compose.yml")
    );
}

#[test]
fn an_app_without_a_compose_file_has_none() {
    let detected = detect(&[("Makefile", C_MAKEFILE)]).unwrap();

    assert_eq!(detected.app.compose_file, None);
}
