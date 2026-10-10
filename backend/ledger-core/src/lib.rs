//! Domain logic of the Ledger Dev Tools extension.
//!
//! Nothing here performs I/O or depends on VS Code: callers read files and pass their content in.
//! "App" always means the Ledger app being built, never this crate.

pub mod actions;
pub mod app;
pub mod c_app;
pub mod command;
pub mod container;
pub mod detect;
pub mod device;
pub mod device_ops;
pub mod docker;
pub mod enforcer;
pub mod environment;
pub mod error;
pub mod failures;
pub mod host_python;
pub mod language;
pub mod manifest;
pub mod platform;
pub mod rust_app;
pub mod selection;
pub mod snapshot;
pub mod target;
pub mod task;
pub mod test_deps;
pub mod test_source;
pub mod test_tree;
pub mod udev;
pub mod update;
pub mod workspace;

pub use command::Command;
pub use device::Device;
pub use error::{Error, Result};
pub use language::Language;
pub use target::Target;

#[cfg(test)]
mod test_support;
