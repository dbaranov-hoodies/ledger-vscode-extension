use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("wrong manifest format: mandatory property \"{0}\" not found")]
    MissingProperty(&'static str),
    #[error("invalid language: {0} in manifest")]
    InvalidLanguage(String),
    #[error("invalid device in manifest: {0}")]
    InvalidDevice(String),
    #[error("manifest lists no devices")]
    NoDevices,
    #[error("file not found in the app folder: {0}")]
    MissingFile(String),
}

pub type Result<T> = std::result::Result<T, Error>;
