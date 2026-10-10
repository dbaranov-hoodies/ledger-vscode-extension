//! The checks of the guideline enforcer, which the CI of the app runs too.

/// What the selection says when every check should run.
pub const ALL: &str = "All";

pub const CHECKS: [&str; 6] = [
    "manifest",
    "icons",
    "app_load_params",
    "makefile",
    "readme",
    "scan",
];
