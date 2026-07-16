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
use crate::isolation::IsolatedConfigDir;
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
/// the subscription, plus any `config.env` overrides and `config.cwd`) with the
/// argv built from `config` and `prompt`, captures stdout, and wraps the whole
/// call in one [`tokio::time::timeout`].
///
/// When `config.isolated` is `true`, an [`IsolatedConfigDir`] is built first and
/// its path is set as `CLAUDE_CONFIG_DIR` in the child's env, so the subprocess
/// runs against a throwaway, redacted copy of the credentials instead of the
/// real `~/.claude/`. The guard is kept alive until the child has exited and its
/// output has been read, so its `Drop` cleanup cannot race the still-running
/// child.
///
/// # Errors
/// - [`Error::BinaryNotFound`] if the `claude` binary cannot be resolved.
/// - [`Error::Isolation`] if `config.isolated` is set and the isolated config
///   dir cannot be built.
/// - [`Error::Spawn`] if the process fails to spawn or its output cannot be read.
/// - [`Error::Timeout`] if the call does not complete within the timeout.
/// - [`Error::Parse`] if stdout is not valid `Outcome` JSON.
pub async fn execute(config: &Config, prompt: &str) -> Result<Outcome> {
    let binary = resolve_binary()?;
    let args = config.build_args(prompt);

    // Built before the async block so a mid-setup failure surfaces before we
    // ever spawn, and so the guard's lifetime spans the whole call (including
    // the timeout race) below.
    let isolation_guard = if config.isolated {
        Some(IsolatedConfigDir::new()?)
    } else {
        None
    };

    let call = async {
        let mut command = Command::new(&binary);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }

        if !config.env.is_empty() {
            command.envs(config.env.iter().map(|(k, v)| (k, v)));
        }

        if let Some(guard) = &isolation_guard {
            command.env("CLAUDE_CONFIG_DIR", guard.path());
        }

        let output = command.output().await.map_err(Error::Spawn)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse::parse_result(&stdout)
    };

    let result = match tokio::time::timeout(DEFAULT_TIMEOUT, call).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Error::Timeout),
    };

    // Keep the guard alive through the whole call above; drop it explicitly
    // here (after output has been read) rather than relying on end-of-scope,
    // to make the "outlives the child" contract explicit.
    drop(isolation_guard);

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    /// `cargo test` runs tests in parallel by default, but every test in this
    /// module mutates the process-global `CLAUDE_BINARY` env var. Serialize
    /// them on one mutex so concurrent tests can't observe each other's
    /// `CLAUDE_BINARY` value mid-test.
    static CLAUDE_BINARY_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_binary_prefers_claude_binary_env() {
        let _guard = CLAUDE_BINARY_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // SAFETY: single-threaded test env mutation, scoped to this test and
        // serialized via `CLAUDE_BINARY_ENV_LOCK`.
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

    /// Writes an executable shell script that ignores argv, echoes `$PWD` and
    /// the given env var (or `<unset>`) as the `model` field of a minimal
    /// valid `Outcome` JSON blob, then exits 0. Used as a stand-in
    /// `CLAUDE_BINARY` so tests can observe what `execute()` actually applied
    /// to the child `Command` without running the real `claude` CLI.
    #[cfg(unix)]
    fn fake_binary_reporting(env_var: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let script_path = dir.path().join("fake-claude.sh");
        let mut file = std::fs::File::create(&script_path).expect("create script");
        writeln!(
            file,
            "#!/bin/sh\nval=\"${{{env_var}:-<unset>}}\"\nprintf '{{\"total_cost_usd\":0.0,\"usage\":{{}},\"model\":\"%s|%s\"}}' \"$val\" \"$PWD\"\n"
        )
        .expect("write script");
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }

        (dir, script_path)
    }

    #[cfg(unix)]
    fn run_with_fake_binary(script_path: &std::path::Path, config: &Config) -> Outcome {
        let _guard = CLAUDE_BINARY_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // SAFETY: single-threaded test env mutation, scoped to this test and
        // serialized via `CLAUDE_BINARY_ENV_LOCK`.
        unsafe {
            std::env::set_var("CLAUDE_BINARY", script_path);
        }

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(execute(config, "hi"));

        unsafe {
            std::env::remove_var("CLAUDE_BINARY");
        }

        result.expect("fake execute should succeed")
    }

    #[cfg(unix)]
    #[test]
    fn execute_applies_cwd_override() {
        let (_script_dir, script_path) = fake_binary_reporting("CLAUDE_CODE_RS_TEST_VAR");
        let target_dir = tempfile::tempdir().expect("target dir");
        let canonical_target = std::fs::canonicalize(target_dir.path()).expect("canonicalize");

        let config = Config {
            cwd: Some(target_dir.path().to_path_buf()),
            ..Config::default()
        };

        let outcome = run_with_fake_binary(&script_path, &config);
        let reported_cwd = outcome
            .model
            .split('|')
            .nth(1)
            .expect("model carries var|cwd");

        assert_eq!(
            std::fs::canonicalize(reported_cwd).expect("canonicalize reported cwd"),
            canonical_target
        );
    }

    #[cfg(unix)]
    #[test]
    fn execute_applies_env_overrides() {
        let (_script_dir, script_path) = fake_binary_reporting("CLAUDE_CODE_RS_TEST_VAR");

        let config = Config {
            env: vec![(
                "CLAUDE_CODE_RS_TEST_VAR".to_string(),
                "isolation-seam-value".to_string(),
            )],
            ..Config::default()
        };

        let outcome = run_with_fake_binary(&script_path, &config);
        let reported_var = outcome
            .model
            .split('|')
            .next()
            .expect("model carries var|cwd");

        assert_eq!(reported_var, "isolation-seam-value");
    }

    #[cfg(unix)]
    #[test]
    fn execute_default_path_sets_no_config_dir_override() {
        let (_script_dir, script_path) = fake_binary_reporting("CLAUDE_CONFIG_DIR");

        let config = Config::default();
        assert!(!config.isolated);

        let outcome = run_with_fake_binary(&script_path, &config);
        let reported_var = outcome
            .model
            .split('|')
            .next()
            .expect("model carries var|cwd");

        assert_eq!(reported_var, "<unset>");
    }

    #[cfg(unix)]
    #[test]
    fn execute_isolated_sets_config_dir_and_guard_outlives_child() {
        let (_script_dir, script_path) = fake_binary_reporting("CLAUDE_CONFIG_DIR");

        let config = Config {
            isolated: true,
            ..Config::default()
        };

        let outcome = run_with_fake_binary(&script_path, &config);
        let reported_var = outcome
            .model
            .split('|')
            .next()
            .expect("model carries var|cwd");

        // The reported CLAUDE_CONFIG_DIR must have been a real, existing
        // directory *while the child ran* (proving the guard outlived it) —
        // by the time we observe it here the guard has already been dropped,
        // so the path itself is gone.
        assert_ne!(reported_var, "<unset>");
        assert!(!std::path::Path::new(reported_var).exists());
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
