//! Process-group teardown helpers (Unix).

use std::time::Duration;

use crate::config::Config;

// Consumed by the timeout-teardown wiring in a later task.
#[allow(dead_code)]
/// Grace period between the polite signal and the forced kill when a process group is torn down.
pub(crate) const DEFAULT_GROUP_KILL_GRACE: Duration = Duration::from_secs(5);

/// Resolve the group-kill grace: `Config::group_kill_grace` if set, else the built-in default.
/// Mirrors `execute::effective_timeout`.
#[must_use]
#[allow(dead_code)]
pub(crate) fn effective_group_kill_grace(config: &Config) -> Duration {
    config.group_kill_grace.unwrap_or(DEFAULT_GROUP_KILL_GRACE)
}

#[cfg(test)]
mod tests;
