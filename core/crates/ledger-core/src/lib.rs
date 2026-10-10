//! Domain logic of the Ledger Dev Tools extension.
//!
//! Nothing here performs I/O or depends on VS Code: callers read files and pass their content in.
//! "App" always means the Ledger app being built, never this crate.

pub mod device;
pub mod error;
pub mod language;
pub mod manifest;
pub mod rust_app;
pub mod target;
pub mod test_tree;

pub use device::Device;
pub use error::{Error, Result};
pub use language::Language;
pub use target::Target;
