//! Async entry point that runs the `claude` CLI as a subprocess and parses its
//! JSON output into an [`Outcome`].
//!
//! Binary resolution: `CLAUDE_BINARY` env var first, else `which::which("claude")`.
//! The whole call (spawn + wait) is wrapped in a single [`tokio::time::timeout`] —
//! no per-line hardcoded timeout — and the child is killed on drop so a timed-out
//! or cancelled call never leaks a subprocess.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::heal;
use crate::isolation::IsolatedConfigDir;
use crate::parse::{self, Outcome};

/// Default whole-call timeout applied to every `execute()` invocation that does
/// not set [`Config::timeout`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Resolve the whole-call timeout for one `execute()` invocation.
///
/// `config.timeout` of `None` — the `Config::default()` value — yields
/// [`DEFAULT_TIMEOUT`], preserving the behavior every existing caller already
/// has; `Some(duration)` overrides it. Kept as a pure function so the
/// resolution rule is unit-testable without spawning a subprocess.
fn effective_timeout(config: &Config) -> Duration {
    config.timeout.unwrap_or(DEFAULT_TIMEOUT)
}

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

/// Spawn `binary` once with `args`, apply `config`'s `cwd`/`env` overrides and
/// `config_dir` (the isolated guard's path, when isolated; `None` on the
/// default, non-isolated path), and parse its output into an [`Outcome`] —
/// wrapped in one [`tokio::time::timeout`] of [`effective_timeout`]`(config)`.
///
/// Behavior-identical to `execute()`'s inline body before this function was
/// extracted from it — this is a pure extraction, not a behavior change — so
/// both the first attempt and the heal-and-retry attempt in `execute()` share
/// one code path instead of two copies.
///
/// # Errors
/// - [`Error::Spawn`] if the process fails to spawn or its output cannot be read.
/// - [`Error::Timeout`] if the call does not complete within the timeout.
/// - [`Error::Cli`] if the CLI produced no output envelope at all (bad argv, missing
///   prompt) — the message is on stderr.
/// - [`Error::Api`] if the CLI reported `is_error` (unroutable model, API outage) —
///   the message is in the envelope, not on stderr.
/// - [`Error::Parse`] if stdout is not valid `Outcome` JSON.
async fn run_once(
    binary: &Path,
    args: &[String],
    config: &Config,
    config_dir: Option<&Path>,
) -> Result<Outcome> {
    let call = async {
        let mut command = Command::new(binary);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }

        if !config.env.is_empty() {
            command.envs(config.env.iter().map(|(k, v)| (k, v)));
        }

        if let Some(dir) = config_dir {
            command.env("CLAUDE_CONFIG_DIR", dir);
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
                session_id: outcome.session_id,
                cost_usd: outcome.cost_usd,
                usage: outcome.usage,
            });
        }

        Ok(outcome)
    };

    match tokio::time::timeout(effective_timeout(config), call).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Error::Timeout),
    }
}

/// Run a single `claude` CLI call and parse its JSON output into an [`Outcome`].
///
/// Spawns `claude` (env inherited from the current process, since auth is free on
/// the subscription, plus any `config.env` overrides and `config.cwd`) with the
/// argv built from `config` and `prompt`, captures stdout, and wraps the whole
/// call in one [`tokio::time::timeout`] of [`effective_timeout`]`(config)` —
/// `config.timeout` when set, else [`DEFAULT_TIMEOUT`] (300s).
///
/// When `config.isolated` is `true`, an [`IsolatedConfigDir`] is built first and
/// its path is set as `CLAUDE_CONFIG_DIR` in the child's env, so the subprocess
/// runs against a throwaway, redacted copy of the credentials instead of the
/// real `~/.claude/`. The guard is kept alive until the child has exited and its
/// output has been read, so its `Drop` cleanup cannot race the still-running
/// child.
///
/// When that first attempt fails with the exact shape of an expired/invalid
/// isolated credential snapshot ([`heal::is_isolated_auth_expired`]) and both
/// `config.isolated` and `config.heal_isolated_auth_on_expiry` are `true`,
/// `execute()` makes one best-effort attempt to heal the real shared
/// credentials ([`heal::heal_shared_credentials`] — its own failure is
/// swallowed, never surfacing in place of the real error), rebuilds a **fresh**
/// [`IsolatedConfigDir`] (the first attempt's guard is stale even after a
/// successful heal, since it snapshotted credentials before the heal ran), and
/// retries exactly once. The retry's `Result` becomes `execute()`'s own result,
/// dropping the original error. This whole path is skipped — and behavior is
/// byte-identical to before it existed — whenever `heal_isolated_auth_on_expiry`
/// is `false` (the default) or the first failure does not match the predicate.
///
/// # Errors
/// - [`Error::ConflictingPermissions`] if `config.dangerously_skip_permissions`
///   and `config.permission_mode` are both set — checked first, before the
///   binary is resolved or anything is spawned.
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
    config.validate()?;
    let binary = resolve_binary()?;
    let args = config.build_args(prompt);

    // Built before the first `run_once` call so a mid-setup failure surfaces
    // before we ever spawn, and so the guard's lifetime spans the whole call
    // (including the timeout race) inside `run_once`.
    //
    // `new_async` (not `new`) because construction shells out to the macOS
    // Keychain and blocks: on `new` that wait runs on this task's own worker
    // thread, so a call here stalls every *other* task on the runtime — a
    // concurrent `execute()` has been observed hitting `Error::Timeout` from
    // this starvation alone, without ever spawning its subprocess.
    let isolation_guard = if config.isolated {
        Some(IsolatedConfigDir::new_async().await?)
    } else {
        None
    };

    let result = run_once(
        &binary,
        &args,
        config,
        isolation_guard.as_ref().map(IsolatedConfigDir::path),
    )
    .await;

    // Keep the guard alive through the call above; drop it explicitly here
    // (after output has been read) rather than relying on end-of-scope, to
    // make the "outlives the child" contract explicit.
    drop(isolation_guard);

    let should_heal_and_retry = config.isolated
        && config.heal_isolated_auth_on_expiry
        && matches!(&result, Err(e) if heal::is_isolated_auth_expired(e));

    if !should_heal_and_retry {
        return result;
    }

    // Best-effort: a heal failure must never replace or mask the real error —
    // if it fails, still attempt the retry, since the shared file may already
    // have healed by other means (e.g. a concurrent interactive session).
    let _ = heal::heal_shared_credentials(&binary, heal::effective_heal_timeout(config)).await;

    // The first attempt's guard is stale even after a successful heal — it
    // snapshotted credentials before the heal ran — so this must be a fresh
    // one, not the dropped `isolation_guard` above.
    let retry_guard = IsolatedConfigDir::new_async().await?;
    let retry_result = run_once(&binary, &args, config, Some(retry_guard.path())).await;
    drop(retry_guard);

    retry_result
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

    #[test]
    fn execute_rejects_conflicting_permissions_before_resolving_binary() {
        let _guard = CLAUDE_BINARY_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // A path that does not exist on disk. If `validate()` did not run
        // first, `resolve_binary()` would still succeed (it only checks the
        // env var is set, not that the path exists) and the subsequent spawn
        // would fail with `Error::Spawn`, not `Error::ConflictingPermissions`.
        //
        // SAFETY: single-threaded test env mutation, scoped to this test and
        // serialized via `CLAUDE_BINARY_ENV_LOCK`.
        unsafe {
            std::env::set_var("CLAUDE_BINARY", "/nonexistent/path/to/claude");
        }

        let config = Config {
            dangerously_skip_permissions: true,
            permission_mode: Some(crate::config::PermissionMode::DontAsk),
            ..Config::default()
        };

        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(execute(&config, "hi"));

        unsafe {
            std::env::remove_var("CLAUDE_BINARY");
        }

        assert!(matches!(result, Err(Error::ConflictingPermissions)));
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
            Error::Api {
                status,
                message,
                session_id,
                ..
            } => {
                assert_eq!(status, Some(404));
                assert_eq!(message, "model not found");
                assert_eq!(
                    session_id, None,
                    "this fixture emits no session_id, so the error must carry None"
                );
            }
            other => panic!("expected Error::Api, got {other:?}"),
        }
    }

    /// A billed failure must stay attributable: the CLI reached the API, consumed tokens, and
    /// reported `is_error` — so the envelope's `session_id` has to survive the conversion into
    /// `Error::Api` rather than being dropped with the discarded `Outcome`. Without this, a cost
    /// comparison silently understates exactly the attempts it most needs to see.
    #[cfg(unix)]
    #[test]
    fn is_error_envelope_carries_its_session_id_into_the_api_error() {
        let (_dir, script_path) = write_fake_binary(
            "printf '{\"total_cost_usd\":0.42,\"usage\":{\"input_tokens\":1200,\"output_tokens\":34},\"modelUsage\":{},\"is_error\":true,\"subtype\":\"success\",\"api_error_status\":529,\"session_id\":\"ffffffff-1111-2222-3333-444444444444\",\"result\":\"overloaded\"}'\nexit 1\n",
        );

        let err = try_run_with_fake_binary(&script_path, &Config::default())
            .expect_err("an is_error envelope must not surface as Ok");

        match err {
            Error::Api {
                session_id,
                cost_usd,
                usage,
                ..
            } => {
                assert_eq!(
                    session_id.as_deref(),
                    Some("ffffffff-1111-2222-3333-444444444444"),
                    "a billed failure must stay joinable to its transcript"
                );
                // The whole point: this attempt was BILLED. The charge and the tokens must survive
                // the conversion, not just the id.
                assert!((cost_usd - 0.42).abs() < 1e-9);
                assert_eq!(usage.input_tokens, 1200);
                assert_eq!(usage.output_tokens, 34);
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

    /// The default-preservation contract: a `Config` that never touches
    /// `timeout` must resolve to exactly today's `DEFAULT_TIMEOUT`, and an
    /// explicit `Some` must be passed through unchanged. Pure — no subprocess,
    /// so it pins the 300s default without waiting 300s to observe it.
    #[test]
    fn effective_timeout_defaults_to_the_constant_and_honors_an_override() {
        assert_eq!(effective_timeout(&Config::default()), DEFAULT_TIMEOUT);
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(300));

        let overridden = Config {
            timeout: Some(Duration::from_millis(50)),
            ..Config::default()
        };
        assert_eq!(effective_timeout(&overridden), Duration::from_millis(50));
    }

    /// The override must actually be wired into the `tokio::time::timeout`
    /// race, not merely resolved and discarded: a fake binary that sleeps 2s
    /// against a 200ms configured timeout must fail fast with
    /// [`Error::Timeout`], well before the sleep could have finished.
    ///
    /// The short 200ms value is deliberate — `try_run_with_fake_binary` holds
    /// `CLAUDE_BINARY_ENV_LOCK` for the whole blocking call, so this test
    /// serializes against the rest of the module. `kill_on_drop(true)` reaps
    /// the still-sleeping child when the timed-out future is dropped.
    #[cfg(unix)]
    #[test]
    fn configured_timeout_fires_before_a_slow_binary_finishes() {
        let (_dir, script_path) = write_fake_binary("sleep 2\nprintf '{}'\n");

        let config = Config {
            timeout: Some(Duration::from_millis(200)),
            ..Config::default()
        };

        let started = std::time::Instant::now();
        let result = try_run_with_fake_binary(&script_path, &config);
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(Error::Timeout)),
            "a 200ms configured timeout against a 2s binary must yield Error::Timeout, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the configured 200ms timeout was not the one that fired — call took {elapsed:?}"
        );
    }

    /// Real captured 401/OAuth-expired envelope, byte-identical to the one
    /// `heal::is_isolated_auth_expired`'s own tests exercise.
    const AUTH_EXPIRED_ENVELOPE: &str =
        include_str!("../tests/fixtures/cli-error-oauth-expired-2.1.270.json");

    /// A minimal valid success envelope.
    const SUCCESS_ENVELOPE: &str =
        r#"{"total_cost_usd":0.0,"usage":{},"is_error":false,"result":"ok"}"#;

    /// Writes a fake `CLAUDE_BINARY` script that tells an `execute()` call
    /// apart from a bare `heal_shared_credentials` call by argv shape, since
    /// `execute()`'s heal-and-retry path spawns the same binary path for both:
    ///
    /// - Invoked with `-p` as its first argument (every real `execute()`
    ///   call, per `Config::build_args`): increments `counter_path` and runs
    ///   `first_body` on the first such invocation, `second_body` on every
    ///   subsequent one.
    /// - Invoked with no `-p` (a bare `heal_shared_credentials` call, which
    ///   spawns the binary with no args at all): appends a marker line to
    ///   `heal_marker_path`, then fails exactly like the real live "no prompt
    ///   provided" case — discarded by `heal_shared_credentials`.
    #[cfg(unix)]
    fn write_counting_binary(
        counter_path: &std::path::Path,
        heal_marker_path: &std::path::Path,
        first_body: &str,
        second_body: &str,
    ) -> (tempfile::TempDir, PathBuf) {
        write_fake_binary(&format!(
            "if [ \"$1\" = \"-p\" ]; then\n  \
             n=$(cat '{counter}' 2>/dev/null || echo 0)\n  \
             n=$((n+1))\n  \
             echo \"$n\" > '{counter}'\n  \
             if [ \"$n\" -eq 1 ]; then\n{first}\n  else\n{second}\n  fi\n\
             else\n  \
             echo x >> '{heal_marker}'\n  \
             echo 'Input must be provided either through stdin or as a prompt argument' >&2\n  \
             exit 1\n\
             fi\n",
            counter = counter_path.display(),
            heal_marker = heal_marker_path.display(),
            first = first_body,
            second = second_body,
        ))
    }

    /// The retry-after-heal path recovers a call that would otherwise have
    /// failed: first `-p` invocation returns the real captured 401/OAuth
    /// envelope, second returns success. With both `config.isolated` and
    /// `config.heal_isolated_auth_on_expiry` true, `execute()` must heal
    /// (proven by the heal marker existing) and retry exactly once (proven by
    /// the counter), returning the retry's `Ok`.
    #[cfg(unix)]
    #[test]
    fn execute_heals_and_retries_once_after_isolated_auth_expiry() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let counter_path = dir.path().join("counter");
        let heal_marker_path = dir.path().join("healed");

        let first_body = format!(
            "printf '%s' '{fixture}'\nexit 1",
            fixture = AUTH_EXPIRED_ENVELOPE
        );
        let second_body = format!(
            "printf '%s' '{success}'\nexit 0",
            success = SUCCESS_ENVELOPE
        );
        let (_script_dir, script_path) =
            write_counting_binary(&counter_path, &heal_marker_path, &first_body, &second_body);

        let config = Config {
            isolated: true,
            heal_isolated_auth_on_expiry: true,
            ..Config::default()
        };

        let outcome = run_with_fake_binary(&script_path, &config);
        assert_eq!(outcome.text, "ok");

        let invocations: u32 = std::fs::read_to_string(&counter_path)
            .expect("counter file should exist")
            .trim()
            .parse()
            .expect("counter file should hold a number");
        assert_eq!(
            invocations, 2,
            "the CLAUDE_BINARY script must be invoked exactly twice: the failing \
             attempt and the successful retry"
        );
        assert!(
            heal_marker_path.exists(),
            "heal_shared_credentials must have invoked the binary between the \
             two execute() attempts"
        );
    }

    /// The default-preservation contract: with `heal_isolated_auth_on_expiry`
    /// left at its default `false`, the identical failing setup must return
    /// the original error on the first failure, with the script invoked
    /// exactly once and heal never attempted — behavior byte-identical to
    /// before this ticket for every caller that does not opt in.
    #[cfg(unix)]
    #[test]
    fn execute_does_not_heal_by_default_and_returns_the_original_error() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let counter_path = dir.path().join("counter");
        let heal_marker_path = dir.path().join("healed");

        let first_body = format!(
            "printf '%s' '{fixture}'\nexit 1",
            fixture = AUTH_EXPIRED_ENVELOPE
        );
        let second_body = format!(
            "printf '%s' '{success}'\nexit 0",
            success = SUCCESS_ENVELOPE
        );
        let (_script_dir, script_path) =
            write_counting_binary(&counter_path, &heal_marker_path, &first_body, &second_body);

        let config = Config {
            isolated: true,
            heal_isolated_auth_on_expiry: false,
            ..Config::default()
        };

        let err = try_run_with_fake_binary(&script_path, &config)
            .expect_err("should hard-fail without healing when opted out");

        match err {
            Error::Api {
                status, message, ..
            } => {
                assert_eq!(status, Some(401));
                assert!(message.contains("OAuth"));
            }
            other => panic!("expected Error::Api, got {other:?}"),
        }

        let invocations: u32 = std::fs::read_to_string(&counter_path)
            .expect("counter file should exist")
            .trim()
            .parse()
            .expect("counter file should hold a number");
        assert_eq!(
            invocations, 1,
            "no retry should be attempted when heal_isolated_auth_on_expiry is false"
        );
        assert!(
            !heal_marker_path.exists(),
            "heal must never be invoked when the caller has not opted in"
        );
    }

    /// Heal not fixing the underlying credential must still surface an error
    /// — from the SECOND attempt, never a stale first-attempt error and never
    /// a third attempt.
    #[cfg(unix)]
    #[test]
    fn execute_returns_second_attempts_error_when_heal_does_not_fix_it() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let counter_path = dir.path().join("counter");
        let heal_marker_path = dir.path().join("healed");

        let same_body = format!(
            "printf '%s' '{fixture}'\nexit 1",
            fixture = AUTH_EXPIRED_ENVELOPE
        );
        let (_script_dir, script_path) =
            write_counting_binary(&counter_path, &heal_marker_path, &same_body, &same_body);

        let config = Config {
            isolated: true,
            heal_isolated_auth_on_expiry: true,
            ..Config::default()
        };

        let err = try_run_with_fake_binary(&script_path, &config)
            .expect_err("heal did not fix the credential, so the call must still fail");
        assert!(
            matches!(
                err,
                Error::Api {
                    status: Some(401),
                    ..
                }
            ),
            "expected the second attempt's Error::Api, got {err:?}"
        );

        let invocations: u32 = std::fs::read_to_string(&counter_path)
            .expect("counter file should exist")
            .trim()
            .parse()
            .expect("counter file should hold a number");
        assert_eq!(
            invocations, 2,
            "exactly one retry attempt — never a stale first error, never a third attempt"
        );
    }

    /// The predicate must gate the whole mechanism: a first failure of a
    /// DIFFERENT shape (no envelope at all — `Error::Cli`) must never trigger
    /// a heal attempt, and the script must be invoked exactly once.
    #[cfg(unix)]
    #[test]
    fn execute_does_not_heal_for_a_non_matching_error_shape() {
        let dir = tempfile::tempdir().expect("scratch dir");
        let counter_path = dir.path().join("counter");
        let heal_marker_path = dir.path().join("healed");

        let cli_error_body = "echo 'error: something else went wrong' >&2\nexit 1";
        let unreachable_body = "echo 'unreachable — no retry should ever run this' >&2\nexit 1";
        let (_script_dir, script_path) = write_counting_binary(
            &counter_path,
            &heal_marker_path,
            cli_error_body,
            unreachable_body,
        );

        let config = Config {
            isolated: true,
            heal_isolated_auth_on_expiry: true,
            ..Config::default()
        };

        let err = try_run_with_fake_binary(&script_path, &config)
            .expect_err("a non-matching error shape must still surface as an error");
        assert!(
            matches!(err, Error::Cli { .. }),
            "expected Error::Cli, got {err:?}"
        );

        let invocations: u32 = std::fs::read_to_string(&counter_path)
            .expect("counter file should exist")
            .trim()
            .parse()
            .expect("counter file should hold a number");
        assert_eq!(
            invocations, 1,
            "the predicate must gate the whole mechanism, not every isolated failure"
        );
        assert!(
            !heal_marker_path.exists(),
            "heal must never be invoked for a non-matching error shape"
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

    /// Resolve the real shared credentials file path — the same
    /// `~/.claude/.credentials.json` resolution `isolation.rs`'s private
    /// `home_dir()`/`read_file_credentials()` use — so this test never
    /// hardcodes a second, independent path to the same file.
    fn shared_credentials_path() -> PathBuf {
        let home = std::env::var_os("HOME").expect("HOME must be set to locate credentials");
        PathBuf::from(home)
            .join(".claude")
            .join(".credentials.json")
    }

    /// Unconditionally restores the shared credentials file to its original
    /// bytes when dropped — including during a panic/assertion-failure
    /// unwind, since `Drop::drop` still runs while unwinding (Rust only skips
    /// it on abort, which this test never triggers). This is what makes it
    /// safe to corrupt a real, live credential file in-process.
    struct CredentialsRestoreGuard {
        path: PathBuf,
        original_bytes: Vec<u8>,
    }

    impl Drop for CredentialsRestoreGuard {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.path, &self.original_bytes);
        }
    }

    /// Live smoke test proving the heal-and-retry mechanism against the real
    /// `claude` binary and the real shared credentials file: corrupts the
    /// live `accessToken`, runs an isolated `execute()` call with healing
    /// opted in, and asserts both that the call recovered and that the
    /// shared file's `accessToken` is no longer the corrupted value.
    ///
    /// Ignored so gated `cargo test` stays green (mirrors
    /// `live_execute_returns_populated_outcome`'s convention exactly) — run
    /// manually with:
    /// `cargo test -- --ignored live_heal_recovers_isolated_call_after_real_credential_corruption`
    /// on a machine with a real subscription login. Never touches
    /// `CLAUDE_BINARY` or any other override — it exercises the real `claude`
    /// resolution path end to end, on purpose.
    #[tokio::test]
    #[ignore]
    async fn live_heal_recovers_isolated_call_after_real_credential_corruption() {
        let creds_path = shared_credentials_path();
        let original_bytes = std::fs::read(&creds_path)
            .expect("real shared credentials file must exist on this machine");

        // Installed before any mutation, so a panic anywhere below (including
        // from the assertions at the end of this test) still restores the
        // original bytes on unwind.
        let _restore_guard = CredentialsRestoreGuard {
            path: creds_path.clone(),
            original_bytes: original_bytes.clone(),
        };

        let original_text =
            String::from_utf8(original_bytes.clone()).expect("credentials file must be UTF-8");
        let mut creds_value: serde_json::Value =
            serde_json::from_str(&original_text).expect("credentials file must be valid JSON");
        let corrupted_access_token = "sk-ant-deliberately-corrupted-by-live-heal-test";
        creds_value["claudeAiOauth"]["accessToken"] =
            serde_json::Value::String(corrupted_access_token.to_string());
        std::fs::write(
            &creds_path,
            serde_json::to_string(&creds_value).expect("corrupted credentials must serialize"),
        )
        .expect("failed to write corrupted credentials for live heal test");

        let config = Config {
            isolated: true,
            heal_isolated_auth_on_expiry: true,
            ..Config::default()
        };

        let outcome = execute(&config, "Say hello in one word.")
            .await
            .expect("heal-and-retry should recover the isolated call");
        assert!(!outcome.is_error);

        let healed_text = std::fs::read_to_string(&creds_path)
            .expect("shared credentials file must still exist after the live call");
        let healed_value: serde_json::Value = serde_json::from_str(&healed_text)
            .expect("healed credentials must still be valid JSON");
        assert_ne!(
            healed_value["claudeAiOauth"]["accessToken"],
            serde_json::Value::String(corrupted_access_token.to_string()),
            "the shared file's accessToken must no longer be the corrupted value after healing"
        );

        // `_restore_guard` drops here, restoring the original bytes.
    }
}
