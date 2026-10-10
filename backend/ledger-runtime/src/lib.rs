//! Runs the commands that `ledger-core` plans, and keeps the Docker containers of an app ready.

pub mod log;

mod cancel;
mod manager;
mod runner;
mod runs;

/// An in-memory Docker, for tests of this crate and of the ones that use it.
#[cfg(any(test, feature = "fake"))]
pub mod fake;

pub use cancel::CancelToken;
pub use log::{Log, LogEvent, LogReceiver, LogSender, Source, Stream};
pub use manager::{Config, ContainerManager, JobOutcome, JobResult, Outcome};
pub use runner::{CommandRunner, Output, SystemRunner, new_run_id};
pub use runs::{ActiveRuns, RunGuard};
