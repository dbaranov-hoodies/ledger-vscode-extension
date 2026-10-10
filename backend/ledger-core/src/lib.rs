//! Domain logic of the Ledger Dev Tools extension.
//!
//! Nothing here performs I/O or depends on VS Code: callers read files and pass their content in.
//! "App" always means the Ledger app being built, never this crate.

pub mod app;
pub mod command;
pub mod container;
pub mod device;
pub mod docker;
pub mod error;
pub mod language;
pub mod manifest;
pub mod platform;
pub mod rust_app;
pub mod target;
pub mod task;
pub mod test_tree;
pub mod update;

pub use command::Command;
pub use device::Device;
pub use error::{Error, Result};
pub use language::Language;
pub use target::Target;
