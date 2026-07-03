//! Async entry point that runs the `claude` CLI as a subprocess and parses its
//! JSON output into an [`Outcome`].
//!
//! Binary resolution: `CLAUDE_BINARY` env var first, else `which::which("claude")`.
//! The whole call (spawn + wait) is wrapped in a single [`tokio::time::timeout`] —
//! no per-line hardcoded timeout — and the child is killed on drop so a timed-out
//! or cancelled call never leaks a subprocess.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::parse::{self, Outcome};

/// Default whole-call timeout applied to every `execute()` invocation.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Resolve the `claude` binary: `CLAUDE_BINARY` env var first, else `PATH` lookup.
///
/// # Errors
/// Returns [`Error::BinaryNotFound`] when neither source resolves.
fn resolve_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CLAUDE_BINARY") {
        return Ok(PathBuf::from(path));
    }

    which::which("claude").map_err(|_| Error::BinaryNotFound)
}

/// Run a single `claude` CLI call and parse its JSON output into an [`Outcome`].
///
/// Spawns `claude` (env inherited from the current process, since auth is free on
/// the subscription) with the argv built from `config` and `prompt`, captures
/// stdout, and wraps the whole call in one [`tokio::time::timeout`].
///
/// # Errors
/// - [`Error::BinaryNotFound`] if the `claude` binary cannot be resolved.
/// - [`Error::Spawn`] if the process fails to spawn or its output cannot be read.
/// - [`Error::Timeout`] if the call does not complete within the timeout.
/// - [`Error::Parse`] if stdout is not valid `Outcome` JSON.
pub async fn execute(config: &Config, prompt: &str) -> Result<Outcome> {
    let binary = resolve_binary()?;
    let args = config.build_args(prompt);

    let call = async {
        let output = Command::new(&binary)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(Error::Spawn)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse::parse_result(&stdout)
    };

    match tokio::time::timeout(DEFAULT_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Error::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_binary_prefers_claude_binary_env() {
        // SAFETY: single-threaded test env mutation, scoped to this test.
        unsafe {
            std::env::set_var("CLAUDE_BINARY", "/usr/bin/env");
        }

        let resolved = resolve_binary();

        unsafe {
            std::env::remove_var("CLAUDE_BINARY");
        }

        assert_eq!(
            resolved.expect("should resolve"),
            PathBuf::from("/usr/bin/env")
        );
    }

    /// Live smoke test — actually runs `execute()` against a trivial prompt on
    /// the subscription. Ignored so gated `cargo test` stays green; run
    /// manually with `cargo test -- --ignored` when `claude` is available.
    #[tokio::test]
    #[ignore]
    async fn live_execute_returns_populated_outcome() {
        let config = Config::default();
        let outcome = execute(&config, "Say hello in one word.")
            .await
            .expect("live execute should succeed");

        assert!(!outcome.model.is_empty());
        assert!(outcome.cost_usd >= 0.0);
    }
}
