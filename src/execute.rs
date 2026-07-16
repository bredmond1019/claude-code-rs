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
/// - [`Error::Cli`] if the CLI produced no output envelope at all (bad argv, missing
///   prompt) — the message is on stderr.
/// - [`Error::Api`] if the CLI reported `is_error` (unroutable model, API outage) —
///   the message is in the envelope, not on stderr.
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

        // Two distinct failure modes, and the exit code alone cannot tell them
        // apart — both exit non-zero (verified against CLI 2.1.211):
        //
        //  * CLI failure (bad flag, missing prompt): stdout empty, message on stderr.
        //  * API failure (unroutable model, outage):  stdout carries a well-formed
        //    envelope with `is_error: true`, and stderr is *empty* — the message
        //    is in the JSON's `result` field.
        //
        // So dispatch on stdout's emptiness, not on the exit status. Reporting
        // stderr for the API case would surface an empty string.
        if stdout.trim().is_empty() {
            return Err(Error::Cli {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let outcome = parse::parse_result(&stdout)?;

        // `is_error` is the only trustworthy signal here: the envelope reports
        // `subtype: "success"` even when the call failed.
        if outcome.is_error {
            return Err(Error::Api {
                status: outcome.api_error_status,
                message: outcome.text,
            });
        }

        Ok(outcome)
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
    /// the given env var (or `<unset>`) as the `result` field of a minimal
    /// valid `Outcome` JSON blob, then exits 0. Used as a stand-in
    /// `CLAUDE_BINARY` so tests can observe what `execute()` actually applied
    /// to the child `Command` without running the real `claude` CLI.
    ///
    /// `result` is the smuggling channel because it is the only required free-text
    /// field on the envelope. (It was `model` until 2026-07-16, when that field
    /// turned out never to have existed — see `tests/fixtures/README.md`.)
    #[cfg(unix)]
    fn fake_binary_reporting(env_var: &str) -> (tempfile::TempDir, PathBuf) {
        write_fake_binary(&format!(
            "val=\"${{{env_var}:-<unset>}}\"\nprintf '{{\"total_cost_usd\":0.0,\"usage\":{{}},\"is_error\":false,\"result\":\"%s|%s\"}}' \"$val\" \"$PWD\"\n"
        ))
    }

    /// Writes an executable `/bin/sh` script with `body` and returns it as a
    /// stand-in `CLAUDE_BINARY`. The `TempDir` must be held for the script's
    /// lifetime.
    #[cfg(unix)]
    fn write_fake_binary(body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let script_path = dir.path().join("fake-claude.sh");
        let mut file = std::fs::File::create(&script_path).expect("create script");
        writeln!(file, "#!/bin/sh\n{body}").expect("write script");
        drop(file);

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod +x");

        (dir, script_path)
    }

    /// As [`run_with_fake_binary`], but returns the `Result` rather than unwrapping —
    /// for the failure-path tests.
    #[cfg(unix)]
    fn try_run_with_fake_binary(script_path: &std::path::Path, config: &Config) -> Result<Outcome> {
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

        result
    }

    #[cfg(unix)]
    fn run_with_fake_binary(script_path: &std::path::Path, config: &Config) -> Outcome {
        try_run_with_fake_binary(script_path, config).expect("fake execute should succeed")
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
            .text
            .split('|')
            .nth(1)
            .expect("result carries var|cwd");

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
            .text
            .split('|')
            .next()
            .expect("result carries var|cwd");

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
            .text
            .split('|')
            .next()
            .expect("result carries var|cwd");

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
            .text
            .split('|')
            .next()
            .expect("result carries var|cwd");

        // The reported CLAUDE_CONFIG_DIR must have been a real, existing
        // directory *while the child ran* (proving the guard outlived it) —
        // by the time we observe it here the guard has already been dropped,
        // so the path itself is gone.
        assert_ne!(reported_var, "<unset>");
        assert!(!std::path::Path::new(reported_var).exists());
    }

    /// The API-failure path: the CLI exits non-zero but emits a well-formed
    /// envelope with `is_error: true`, and **stderr is empty** — so the message
    /// must come from the envelope's `result`. Mirrors a real capture of
    /// `claude -p hi --model does-not-exist-xyz` against CLI 2.1.211.
    #[cfg(unix)]
    #[test]
    fn envelope_reporting_is_error_becomes_api_error_with_message_from_result() {
        let (_dir, script_path) = write_fake_binary(
            "printf '{\"total_cost_usd\":0,\"usage\":{},\"modelUsage\":{},\"is_error\":true,\"subtype\":\"success\",\"api_error_status\":404,\"result\":\"model not found\"}'\nexit 1\n",
        );

        let err = try_run_with_fake_binary(&script_path, &Config::default())
            .expect_err("an is_error envelope must not surface as Ok");

        match err {
            Error::Api { status, message } => {
                assert_eq!(status, Some(404));
                assert_eq!(message, "model not found");
            }
            other => panic!("expected Error::Api, got {other:?}"),
        }
    }

    /// `subtype` reports `"success"` on the error envelope, so it must never be
    /// the thing we branch on. This pins that: the fixture above says
    /// `subtype: "success"` *and* `is_error: true`, and `is_error` must win.
    #[cfg(unix)]
    #[test]
    fn is_error_wins_over_a_subtype_claiming_success() {
        let (_dir, script_path) = write_fake_binary(
            "printf '{\"total_cost_usd\":0,\"usage\":{},\"is_error\":true,\"subtype\":\"success\",\"result\":\"boom\"}'\n",
        );

        assert!(
            matches!(
                try_run_with_fake_binary(&script_path, &Config::default()),
                Err(Error::Api { .. })
            ),
            "`subtype: success` must not mask `is_error: true`"
        );
    }

    /// The CLI-failure path: no envelope at all (bad argv, missing prompt).
    /// stdout is empty and the message is on stderr. Mirrors a real capture of
    /// `claude -p hi --bogus-flag-xyz`.
    #[cfg(unix)]
    #[test]
    fn empty_stdout_becomes_cli_error_carrying_stderr() {
        let (_dir, script_path) =
            write_fake_binary("echo \"error: unknown option '--bogus'\" >&2\nexit 1\n");

        let err = try_run_with_fake_binary(&script_path, &Config::default())
            .expect_err("an empty stdout must not surface as Ok");

        match err {
            Error::Cli { status, stderr } => {
                assert_eq!(status, Some(1));
                assert_eq!(stderr, "error: unknown option '--bogus'");
            }
            other => panic!("expected Error::Cli, got {other:?}"),
        }
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

        // The regression guard for the 2026-07-16 silent-data-loss drift: when the
        // CLI moved response text from `content` blocks to `result`, the parser kept
        // returning a clean, empty Outcome. Assert the text is actually there.
        assert!(
            !outcome.text.is_empty(),
            "live call returned empty text — the response-text field has drifted again"
        );
        assert!(
            outcome.primary_model().is_some(),
            "live success envelope must report at least one modelUsage entry"
        );
        assert!(outcome.cost_usd >= 0.0);
        assert!(!outcome.is_error);
    }
}
