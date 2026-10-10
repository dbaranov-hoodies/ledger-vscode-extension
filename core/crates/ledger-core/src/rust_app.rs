//! The `Cargo.toml` of the Rust app being built (not of this crate) and the names it declares.

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustAppPackage {
    /// `package.name`: the crate and artifact name.
    pub package_name: String,
    /// `package.metadata.ledger.name`: the name shown on the device.
    pub app_name: String,
}

pub fn parse_cargo_toml(source: &str) -> Result<RustAppPackage> {
    let raw: RawCargo = toml::from_str(source)?;
    let package = raw.package.unwrap_or_default();
    let package_name = package.name.ok_or(Error::MissingProperty("package.name"))?;
    let app_name = package
        .metadata
        .and_then(|metadata| metadata.ledger)
        .and_then(|ledger| ledger.name)
        .ok_or(Error::MissingProperty("package.metadata.ledger.name"))?;
    Ok(RustAppPackage {
        package_name,
        app_name,
    })
}

#[derive(Deserialize)]
struct RawCargo {
    package: Option<RawPackage>,
}

#[derive(Deserialize, Default)]
struct RawPackage {
    name: Option<String>,
    metadata: Option<RawMetadata>,
}

#[derive(Deserialize)]
struct RawMetadata {
    ledger: Option<RawLedger>,
}

#[derive(Deserialize)]
struct RawLedger {
    name: Option<String>,
}
