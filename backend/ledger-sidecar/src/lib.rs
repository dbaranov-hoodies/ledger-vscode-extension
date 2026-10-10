//! The sidecar of the Ledger Dev Tools extension. It owns the state of the apps and runs what the
//! user starts; the extension only talks to VS Code and to the user.

pub mod protocol;
mod pytest_cache;
pub mod server;
pub mod wire;

pub use server::Server;
