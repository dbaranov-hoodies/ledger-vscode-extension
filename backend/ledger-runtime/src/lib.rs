//! Runs the commands that `ledger-core` plans, and keeps the Docker containers of an app ready.

pub mod log;

mod manager;
mod runner;
mod runs;

#[cfg(test)]
mod fake;

pub use log::{Log, LogEvent, LogReceiver, LogSender, Source, Stream};
pub use manager::{Config, ContainerManager, JobOutcome, JobResult, Outcome};
pub use runner::{CommandRunner, Output, SystemRunner, new_run_id};
pub use runs::{ActiveRuns, RunGuard};
