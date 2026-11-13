//! Process-group teardown helpers (Unix).

use std::time::Duration;

use crate::config::Config;

/// Grace period between the polite signal and the forced kill when a process group is torn down.
pub(crate) const DEFAULT_GROUP_KILL_GRACE: Duration = Duration::from_secs(5);

/// Resolve the group-kill grace: `Config::group_kill_grace` if set, else the built-in default.
/// Mirrors `execute::effective_timeout`.
#[must_use]
pub(crate) fn effective_group_kill_grace(config: &Config) -> Duration {
    config.group_kill_grace.unwrap_or(DEFAULT_GROUP_KILL_GRACE)
}

/// Send `sig` to every process in group `pgid`; `ESRCH` (group already gone) and any other error
/// are ignored — teardown is best-effort.
fn signal_group(pgid: i32, sig: i32) {
    if pgid <= 0 {
        return;
    }
    // SAFETY: `kill` has no memory-safety preconditions; a negative pid targets the group.
    unsafe {
        libc::kill(-pgid, sig);
    }
}

/// Tears down a child's whole process group (the child was spawned with `process_group(0)`, so
/// its pid is the pgid). Call [`GroupKillGuard::disarm`] on normal completion; otherwise dropping
/// the guard (timeout, cancelled future) SIGTERMs the group and, after the grace, SIGKILLs it.
#[derive(Debug)]
pub(crate) struct GroupKillGuard {
    pgid: i32,
    grace: Duration,
    armed: bool,
}

impl GroupKillGuard {
    /// Guard group `pgid` (the child's pid), escalating to SIGKILL after `grace`.
    #[must_use]
    pub(crate) fn new(pgid: u32, grace: Duration) -> Self {
        Self {
            pgid: i32::try_from(pgid).unwrap_or(0),
            grace,
            armed: true,
        }
    }

    /// Normal completion: the guard will no longer signal anything.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    /// SIGTERM the group, wait the grace, then SIGKILL it. Disarms the guard.
    #[allow(dead_code)]
    pub(crate) async fn terminate(mut self) {
        self.armed = false;
        signal_group(self.pgid, libc::SIGTERM);
        tokio::time::sleep(self.grace).await;
        signal_group(self.pgid, libc::SIGKILL);
    }
}

impl Drop for GroupKillGuard {
    /// No async context available: SIGTERM now, SIGKILL from a detached thread after the grace.
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (pgid, grace) = (self.pgid, self.grace);
        signal_group(pgid, libc::SIGTERM);
        // If the thread cannot be spawned, fall back to an immediate SIGKILL.
        if std::thread::Builder::new()
            .name("group-kill".into())
            .spawn(move || {
                std::thread::sleep(grace);
                signal_group(pgid, libc::SIGKILL);
            })
            .is_err()
        {
            signal_group(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests;
