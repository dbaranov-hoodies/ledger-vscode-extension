use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Language of the app being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    C,
    Rust,
}

impl Language {
    /// Parses the `app.sdk` value of a manifest, ignoring case.
    pub fn from_manifest(value: &str) -> Result<Self> {
        if value.eq_ignore_ascii_case("c") {
            Ok(Self::C)
        } else if value.eq_ignore_ascii_case("rust") {
            Ok(Self::Rust)
        } else {
            Err(Error::InvalidLanguage(value.to_owned()))
        }
    }
}
