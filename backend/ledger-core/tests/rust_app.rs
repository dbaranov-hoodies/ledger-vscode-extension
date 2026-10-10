use ledger_core::Error;
use ledger_core::rust_app::{self, RustAppPackage};

const CARGO_TOML: &str = r#"
[package]
name = "app-boilerplate-rust"
version = "0.1.0"

[package.metadata.ledger]
name = "Boilerplate"
curve = ["secp256k1"]
flags = "0"
path = ["44'/1'"]
"#;

#[test]
fn reads_the_package_name_and_the_app_name() {
    assert_eq!(
        rust_app::parse_cargo_toml(CARGO_TOML).unwrap(),
        RustAppPackage {
            package_name: "app-boilerplate-rust".to_owned(),
            app_name: "Boilerplate".to_owned(),
        }
    );
}

#[test]
fn package_name_is_mandatory() {
    let error = rust_app::parse_cargo_toml("[package]\nversion = \"0.1.0\"\n").unwrap_err();

    assert!(matches!(error, Error::MissingProperty("package.name")));
}

#[test]
fn ledger_app_name_is_mandatory() {
    let error = rust_app::parse_cargo_toml("[package]\nname = \"app\"\n").unwrap_err();

    assert!(matches!(
        error,
        Error::MissingProperty("package.metadata.ledger.name")
    ));
}

#[test]
fn missing_package_table_is_reported_as_missing_name() {
    assert!(matches!(
        rust_app::parse_cargo_toml("").unwrap_err(),
        Error::MissingProperty("package.name")
    ));
}

#[test]
fn malformed_toml_is_rejected() {
    assert!(matches!(
        rust_app::parse_cargo_toml("[package\nname = ").unwrap_err(),
        Error::Toml(_)
    ));
}
