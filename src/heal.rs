//! Best-effort recovery for an isolated call whose frozen credential snapshot has gone stale.
//!
//! [`crate::isolation::IsolatedConfigDir`] deliberately strips `refreshToken` from the copy it
//! hands an isolated subprocess, so that subprocess can never consume the single-use refresh
//! token and revoke a concurrent interactive session's credentials. The consequence: an isolated
//! call can never self-heal an expired or invalid access token on its own — it only ever gets a
//! frozen snapshot at copy time. This module identifies that exact failure shape
//! ([`is_isolated_auth_expired`]) and, when the caller opts in, provides the recovery mechanism
//! ([`heal_shared_credentials`], wired into [`crate::execute::execute`]).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::config::Config;
use crate::error::{Error, Result};

/// Default whole-call timeout applied to [`heal_shared_credentials`] when the caller does not set
/// [`Config::heal_timeout`].
// Not yet read from `execute()` — that wiring lands in a later task of this ticket (see the
// matching note on `is_isolated_auth_expired` below). This module's own tests already exercise it.
#[allow(dead_code)]
pub(crate) const DEFAULT_HEAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Resolve the whole-call timeout for one [`heal_shared_credentials`] invocation.
///
/// `config.heal_timeout` of `None` — the `Config::default()` value — yields
/// [`DEFAULT_HEAL_TIMEOUT`]; `Some(duration)` overrides it. Mirrors
/// `execute::effective_timeout`'s shape exactly.
#[allow(dead_code)]
pub(crate) fn effective_heal_timeout(config: &Config) -> Duration {
    config.heal_timeout.unwrap_or(DEFAULT_HEAL_TIMEOUT)
}

/// Best-effort recovery for the real, shared credentials file underneath every isolated call.
///
/// Spawns `binary` with **no** `CLAUDE_CONFIG_DIR` override, so it reads and, if needed, refreshes
/// the real shared credentials — including the `refreshToken` that [`crate::isolation::IsolatedConfigDir`]
/// deliberately strips from every isolated copy. The spawned process is given no prompt and no
/// stdin, so it is expected to perform its OAuth handshake and then fail on the missing prompt;
/// that failure (and any output) is always discarded and treated as success here — only a spawn
/// failure or the wrapping timeout elapsing are this function's own errors.
///
/// # Errors
/// - [`Error::Spawn`] if the process fails to spawn.
/// - [`Error::Timeout`] if the process does not exit within `timeout`.
// Not yet called from `execute()` — that wiring lands in a later task of this ticket.
#[allow(dead_code)]
pub(crate) async fn heal_shared_credentials(binary: &Path, timeout: Duration) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let call = async { command.output().await.map_err(Error::Spawn) };

    match tokio::time::timeout(timeout, call).await {
        Ok(Ok(_output)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => Err(Error::Timeout),
    }
}

/// Returns `true` only when `err` is the exact failure shape of an isolated call whose credential
/// snapshot has gone stale: an [`Error::Api`] with HTTP status `401` whose message contains the
/// substring `"OAuth"`.
///
/// This is deliberately narrow. Every other [`Error`] variant, and every other `Error::Api` shape
/// (a non-401 status, or a 401 with an unrelated message), returns `false` — the predicate must
/// gate the whole heal-and-retry mechanism to this one recoverable case, not to every isolated
/// failure.
// Not yet called from `execute()` — that wiring lands in a later task of this ticket. Allowed
// here (rather than left to trip `-D warnings`) because this module's own tests already exercise
// it against the real captured fixture; removing this attribute is part of that later task.
#[allow(dead_code)]
#[must_use]
pub(crate) fn is_isolated_auth_expired(err: &Error) -> bool {
    matches!(
        err,
        Error::Api {
            status: Some(401),
            message,
            ..
        } if message.contains("OAuth")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use std::io::Write;

    const FIXTURE: &str = include_str!("../tests/fixtures/cli-error-oauth-expired-2.1.270.json");

    /// Writes an executable `/bin/sh` script with `body` and returns it as a
    /// stand-in binary. The `TempDir` must be held for the script's lifetime.
    /// Mirrors `execute.rs`'s own `write_fake_binary` test helper.
    #[cfg(unix)]
    fn write_fake_binary(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
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

    #[test]
    fn effective_heal_timeout_defaults_to_the_constant_and_honors_an_override() {
        assert_eq!(
            effective_heal_timeout(&Config::default()),
            DEFAULT_HEAL_TIMEOUT
        );
        assert_eq!(DEFAULT_HEAL_TIMEOUT, Duration::from_secs(15));

        let overridden = Config {
            heal_timeout: Some(Duration::from_millis(50)),
            ..Config::default()
        };
        assert_eq!(
            effective_heal_timeout(&overridden),
            Duration::from_millis(50)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn heal_shared_credentials_times_out_at_the_configured_duration_not_the_default() {
        let (_dir, script_path) = write_fake_binary("sleep 2\nprintf 'irrelevant'\n");

        let started = std::time::Instant::now();
        let result = heal_shared_credentials(&script_path, Duration::from_millis(200)).await;
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(Error::Timeout)),
            "expected Error::Timeout, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "the configured 200ms timeout was not the one that fired — call took {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn heal_shared_credentials_discards_an_immediate_nonzero_exit() {
        let (_dir, script_path) = write_fake_binary(
            "echo 'Input must be provided either through stdin or as a prompt argument' >&2\nexit 1\n",
        );

        let result = heal_shared_credentials(&script_path, Duration::from_secs(5)).await;

        assert!(
            result.is_ok(),
            "the bare call's own failure/exit code must be discarded, got {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn heal_shared_credentials_kills_a_never_terminating_binary_within_the_timeout() {
        let (_dir, script_path) = write_fake_binary("while true; do sleep 1; done\n");

        let started = std::time::Instant::now();
        let result = heal_shared_credentials(&script_path, Duration::from_millis(200)).await;
        let elapsed = started.elapsed();

        assert!(
            matches!(result, Err(Error::Timeout)),
            "expected Error::Timeout, got {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "kill_on_drop should reap the still-running child promptly, took {elapsed:?}"
        );
    }

    /// Builds the exact `Error::Api` shape `execute()` itself produces from an `is_error`
    /// envelope, so this test exercises the real conversion path rather than a hand-built value.
    fn api_error_from_envelope(json: &str) -> Error {
        let outcome = parse::parse_result(json).expect("fixture must parse");
        assert!(outcome.is_error, "fixture must be an error envelope");
        Error::Api {
            status: outcome.api_error_status,
            message: outcome.text,
            session_id: outcome.session_id,
            cost_usd: outcome.cost_usd,
            usage: outcome.usage,
        }
    }

    #[test]
    fn true_for_the_real_captured_fixture() {
        let err = api_error_from_envelope(FIXTURE);
        assert!(
            is_isolated_auth_expired(&err),
            "the real captured 401/OAuth envelope must match, got {err:?}"
        );
    }

    #[test]
    fn true_for_the_second_real_production_message_text() {
        let err = Error::Api {
            status: Some(401),
            message: "Failed to authenticate. API Error: 401 OAuth access token has expired. Re-authenticate to continue.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: parse::Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        assert!(is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_timeout() {
        assert!(!is_isolated_auth_expired(&Error::Timeout));
    }

    #[test]
    fn false_for_binary_not_found() {
        assert!(!is_isolated_auth_expired(&Error::BinaryNotFound));
    }

    #[test]
    fn false_for_cli_error() {
        let err = Error::Cli {
            status: Some(1),
            stderr: "anything at all, including the word OAuth".to_string(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_non_401_status_with_oauth_message() {
        let err = Error::Api {
            status: Some(500),
            message: "OAuth access token is invalid.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: zero_usage(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_401_status_without_oauth_in_message() {
        let err = Error::Api {
            status: Some(401),
            message: "Unauthorized.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: zero_usage(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    fn zero_usage() -> parse::Usage {
        parse::Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }
}
