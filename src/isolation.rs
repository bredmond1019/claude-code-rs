//! Isolated `CLAUDE_CONFIG_DIR` temp directory for concurrent subprocess auth.
//!
//! Interactive `claude` sessions and SDK-driven subprocess calls share the same
//! `~/.claude/.credentials.json` by default. If a subprocess call happens to
//! refresh its OAuth token mid-flight, the single-use refresh token is consumed
//! server-side and the interactive session's stored credentials are silently
//! revoked. [`IsolatedConfigDir`] sidesteps this: it builds a throwaway
//! `CLAUDE_CONFIG_DIR` containing a *redacted* copy of the credentials (the
//! `refreshToken` field deleted, so the subprocess's refresh check
//! short-circuits) plus a copy of `.claude.json` when present, and removes the
//! whole directory on drop.
//!
//! Ported from the reference Python SDK's
//! `claude_agent_sdk._internal.session_resume` (`_copy_auth_files`,
//! `_write_redacted_credentials`, `_read_keychain_credentials`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;

use crate::error::{Error, Result};

/// macOS Keychain service name under which the `claude` CLI stores OAuth
/// credentials when `CLAUDE_CONFIG_DIR` is unset.
const KEYCHAIN_SERVICE_NAME: &str = "Claude Code-credentials";

/// How many times to re-read a credential source that is present but
/// carries no usable access token before giving up.
///
/// Observed live: a concurrent `claude` CLI OAuth refresh can momentarily
/// leave both the Keychain and the `~/.claude/.credentials.json` fallback
/// holding a placeholder blob (`accessToken: ""`, `expiresAt: 0`) rather
/// than the real, valid one — parseable JSON, present OAuth object, empty
/// token. Accepting that blob at face value silently ships an isolated
/// subprocess a credential that Anthropic's API will reject, surfacing as a
/// confusing remote "not logged in" several layers away from the real,
/// local, transient cause. Retrying the *read* (not the whole subprocess
/// call) closes the window without spending the caller's own transport
/// retry budget on what is actually a local timing issue.
const CREDENTIAL_READ_RETRY_ATTEMPTS: u32 = 4;

/// Delay between credential-read retries. Chosen empirically to be long
/// enough that a self-healing refresh has settled by the next attempt.
const CREDENTIAL_READ_RETRY_DELAY: Duration = Duration::from_millis(150);

/// RAII guard for a temp directory laid out like `~/.claude/`, suitable for
/// pointing an isolated subprocess at via `CLAUDE_CONFIG_DIR`.
///
/// The directory (and everything in it, including the redacted credentials)
/// is removed when the guard is dropped.
#[derive(Debug)]
pub struct IsolatedConfigDir {
    dir: TempDir,
}

impl IsolatedConfigDir {
    /// Build an isolated config dir using the real credential sources: the
    /// macOS Keychain first (best-effort — any failure falls through), then
    /// the file fallback `~/.claude/.credentials.json`. `.claude.json` is
    /// copied from `~/.claude.json` when present.
    ///
    /// This is a **blocking** constructor: the Keychain lookup shells out to
    /// `security` and waits for it. Never call it directly from an async task —
    /// use [`IsolatedConfigDir::new_async`], which runs it on tokio's blocking
    /// pool.
    ///
    /// # Errors
    /// Returns [`Error::Isolation`] if the temp directory cannot be created or
    /// a source file exists but cannot be copied.
    pub fn new() -> Result<Self> {
        Self::new_with(read_keychain_credentials, read_file_credentials)
    }

    /// Async wrapper over [`IsolatedConfigDir::new`], run on tokio's blocking
    /// thread pool via [`tokio::task::spawn_blocking`].
    ///
    /// `new()` shells out to the macOS Keychain (`security find-generic-password`)
    /// and blocks for as long as that takes — and macOS's `securityd` serializes
    /// concurrent keychain reads, so under concurrency that wait is not short.
    /// Calling it directly from an async task parks a tokio *worker* thread for
    /// the whole duration, starving every other task on that runtime; a
    /// concurrent `execute()` call has been observed timing out for this reason
    /// alone, having never spawned its own subprocess. This wrapper moves the
    /// wait onto the blocking pool, where blocking is what the threads are for.
    ///
    /// Behavior is otherwise identical to [`IsolatedConfigDir::new`], including
    /// the best-effort credential fallback chain.
    ///
    /// # Errors
    /// The same [`Error::Isolation`] cases as [`IsolatedConfigDir::new`], plus
    /// an [`Error::Isolation`] wrapping a [`tokio::task::JoinError`] if the
    /// blocking task panicked or was cancelled.
    pub async fn new_async() -> Result<Self> {
        Self::spawn_blocking_build(Self::new).await
    }

    /// Shared constructor body, with both credential readers injected so
    /// tests can substitute deliberately slow or deliberately-empty ones
    /// without touching the real Keychain or `~/.claude/.credentials.json`.
    ///
    /// # Errors
    /// Returns [`Error::Isolation`] if a source is present but never yields
    /// a usable access token within [`CREDENTIAL_READ_RETRY_ATTEMPTS`] —
    /// see [`read_credentials_with_retry`].
    fn new_with(
        read_keychain: impl Fn() -> Option<String>,
        read_file: impl Fn() -> Option<String>,
    ) -> Result<Self> {
        let creds_json = read_credentials_with_retry(&read_keychain, &read_file)?;
        let claude_json_src = home_dir().map(|home| home.join(".claude.json"));
        Self::build(None, creds_json, claude_json_src.as_deref())
    }

    /// Run a blocking construction on tokio's blocking pool, mapping a join
    /// failure (panic/cancellation) into [`Error::Isolation`].
    async fn spawn_blocking_build(
        build: impl FnOnce() -> Result<Self> + Send + 'static,
    ) -> Result<Self> {
        tokio::task::spawn_blocking(build)
            .await
            .map_err(|e| Error::Isolation(io::Error::other(e)))?
    }

    /// Test-only async seam mirroring [`IsolatedConfigDir::new_async`] exactly,
    /// but with the (real, slow, machine-dependent) Keychain reader replaced by
    /// an injected one. Lets the concurrency tests below assert *where* the
    /// blocking read runs without ever touching the real Keychain.
    #[cfg(test)]
    async fn new_async_with(
        read_keychain: impl Fn() -> Option<String> + Send + 'static,
        read_file: impl Fn() -> Option<String> + Send + 'static,
    ) -> Result<Self> {
        Self::spawn_blocking_build(move || Self::new_with(read_keychain, read_file)).await
    }

    /// Test/injection seam: build an isolated config dir from an explicit
    /// credentials JSON string and an explicit `.claude.json` source path,
    /// bypassing the Keychain and `~/.claude/` entirely.
    ///
    /// `creds_json` of `None` skips writing `.credentials.json`.
    /// `claude_json_src` of `None`, or a path that does not exist, skips
    /// copying `.claude.json`.
    ///
    /// # Errors
    /// Returns [`Error::Isolation`] if the temp directory cannot be created or
    /// `claude_json_src` exists but cannot be read/copied.
    pub fn with_sources(
        creds_json: Option<String>,
        claude_json_src: Option<&Path>,
    ) -> Result<Self> {
        Self::build(None, creds_json, claude_json_src)
    }

    /// Absolute path to the temp `CLAUDE_CONFIG_DIR`.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Shared constructor. `parent` is a test-only seam letting unit tests
    /// create the temp dir under a known, empty scratch directory so they can
    /// assert nothing is left behind on a mid-construction failure.
    fn build(
        parent: Option<&Path>,
        creds_json: Option<String>,
        claude_json_src: Option<&Path>,
    ) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("claude-code-rs-");
        let dir = match parent {
            Some(p) => builder.tempdir_in(p),
            None => builder.tempdir(),
        }
        .map_err(Error::Isolation)?;

        if let Some(json) = creds_json {
            write_redacted_credentials(&json, &dir.path().join(".credentials.json"))
                .map_err(Error::Isolation)?;
        }

        if let Some(src) = claude_json_src {
            copy_if_present(src, &dir.path().join(".claude.json")).map_err(Error::Isolation)?;
        }

        Ok(Self { dir })
    }
}

/// Write `creds_json` to `dst` with `claudeAiOauth.refreshToken` deleted
/// (mode `0600` on unix). Unparseable JSON is written through unchanged,
/// mirroring the Python reference — the subprocess will fail to parse it too.
fn write_redacted_credentials(creds_json: &str, dst: &Path) -> io::Result<()> {
    let out = match serde_json::from_str::<serde_json::Value>(creds_json) {
        Ok(mut value) => {
            if let Some(oauth) = value
                .get_mut("claudeAiOauth")
                .and_then(|v| v.as_object_mut())
            {
                oauth.remove("refreshToken");
            }
            serde_json::to_string(&value).unwrap_or_else(|_| creds_json.to_string())
        }
        Err(_) => creds_json.to_string(),
    };

    std::fs::write(dst, out)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Copy `src` to `dst`, skipping silently when `src` does not exist.
/// Any other I/O error (permission denied, `src` is a directory, ...)
/// propagates.
fn copy_if_present(src: &Path, dst: &Path) -> io::Result<()> {
    match std::fs::read(src) {
        Ok(bytes) => std::fs::write(dst, bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Best-effort macOS Keychain lookup for the `claude` CLI's OAuth credentials.
/// Returns `None` on any error, including on non-macOS platforms.
fn read_keychain_credentials() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }

    let user = std::env::var("USER").ok()?;
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &user,
            "-w",
            "-s",
            KEYCHAIN_SERVICE_NAME,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// File fallback for OAuth credentials: `~/.claude/.credentials.json`.
fn read_file_credentials() -> Option<String> {
    let path = home_dir()?.join(".claude").join(".credentials.json");
    std::fs::read_to_string(path).ok()
}

/// Outcome of probing one credential source.
enum Probe {
    /// The source produced nothing — not configured, or genuinely
    /// unreachable. Never retried: there is no reason to expect the next
    /// call to differ.
    Absent,
    /// The source produced JSON with no usable `claudeAiOauth.accessToken`
    /// — the transient placeholder shape a concurrent refresh can leave
    /// behind. Worth retrying.
    Invalid(String),
    /// A real, usable OAuth blob.
    Valid(String),
}

/// True iff `json` parses and its `claudeAiOauth.accessToken` is a
/// non-empty string. Unparseable JSON and a missing/empty token are both
/// "not usable" — the redaction step further down handles unparseable JSON
/// by writing it through as-is (mirroring the Python reference), which is
/// orthogonal to this check.
fn has_usable_access_token(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.get("claudeAiOauth")?
                .get("accessToken")?
                .as_str()
                .map(str::to_owned)
        })
        .is_some_and(|token| !token.is_empty())
}

fn probe(raw: Option<String>) -> Probe {
    match raw {
        None => Probe::Absent,
        Some(json) if has_usable_access_token(&json) => Probe::Valid(json),
        Some(json) => Probe::Invalid(json),
    }
}

/// One round of the existing Keychain-then-file fallback, but distinguishing
/// "no source at all" (proceed with no credentials — a legitimate,
/// unretried case) from "a source exists but is transiently unusable"
/// (worth retrying).
fn probe_round(
    read_keychain: &impl Fn() -> Option<String>,
    read_file: &impl Fn() -> Option<String>,
) -> Probe {
    match probe(read_keychain()) {
        Probe::Valid(json) => Probe::Valid(json),
        Probe::Absent => probe(read_file()),
        Probe::Invalid(keychain_json) => match probe(read_file()) {
            Probe::Valid(json) => Probe::Valid(json),
            _ => Probe::Invalid(keychain_json),
        },
    }
}

/// Read OAuth credentials via the Keychain-then-file fallback, retrying a
/// bounded number of times when a source is present but transiently
/// unusable (see [`CREDENTIAL_READ_RETRY_ATTEMPTS`]).
///
/// # Errors
/// Returns [`Error::Isolation`] if every attempt found a source present but
/// never usable. `Ok(None)` is the distinct, legitimate "no credentials
/// configured anywhere" case — never retried, never an error.
fn read_credentials_with_retry(
    read_keychain: &impl Fn() -> Option<String>,
    read_file: &impl Fn() -> Option<String>,
) -> Result<Option<String>> {
    for attempt in 0..CREDENTIAL_READ_RETRY_ATTEMPTS {
        match probe_round(read_keychain, read_file) {
            Probe::Valid(json) => return Ok(Some(json)),
            Probe::Absent => return Ok(None),
            Probe::Invalid(_) if attempt + 1 < CREDENTIAL_READ_RETRY_ATTEMPTS => {
                std::thread::sleep(CREDENTIAL_READ_RETRY_DELAY);
            }
            Probe::Invalid(_) => {}
        }
    }

    Err(Error::Isolation(io::Error::other(format!(
        "OAuth credentials source present but had no usable access token after \
         {CREDENTIAL_READ_RETRY_ATTEMPTS} attempts over {:?} — likely a concurrent \
         token refresh; retry once it settles",
        CREDENTIAL_READ_RETRY_DELAY * (CREDENTIAL_READ_RETRY_ATTEMPTS - 1)
    ))))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Stands in for a real macOS Keychain read under contention — `securityd`
    /// serializes concurrent lookups, so this is a slow call, not a fast one.
    /// Long enough that blocking a worker thread for it is unmistakable in the
    /// tests below, short enough not to drag the suite.
    const SLOW_KEYCHAIN_READ: Duration = Duration::from_millis(300);

    #[test]
    fn redacts_refresh_token_preserving_other_fields() {
        let creds =
            r#"{"claudeAiOauth":{"accessToken":"abc","refreshToken":"secret","expiresAt":123}}"#;

        let guard = IsolatedConfigDir::with_sources(Some(creds.to_string()), None)
            .expect("build should succeed");

        let written = std::fs::read_to_string(guard.path().join(".credentials.json"))
            .expect("credentials file should exist");
        let value: serde_json::Value =
            serde_json::from_str(&written).expect("written creds should still be valid JSON");

        assert!(value["claudeAiOauth"].get("refreshToken").is_none());
        assert_eq!(value["claudeAiOauth"]["accessToken"], "abc");
        assert_eq!(value["claudeAiOauth"]["expiresAt"], 123);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(guard.path().join(".credentials.json"))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn unparseable_credentials_are_written_through_as_is() {
        let creds = "not valid json";

        let guard = IsolatedConfigDir::with_sources(Some(creds.to_string()), None)
            .expect("build should succeed");

        let written = std::fs::read_to_string(guard.path().join(".credentials.json"))
            .expect("credentials file should exist");
        assert_eq!(written, creds);
    }

    #[test]
    fn no_credentials_source_skips_credentials_file() {
        let guard = IsolatedConfigDir::with_sources(None, None).expect("build should succeed");
        assert!(!guard.path().join(".credentials.json").exists());
    }

    #[test]
    fn copies_claude_json_when_present() {
        let src_dir = tempfile::tempdir().expect("scratch dir");
        let src_path = src_dir.path().join(".claude.json");
        std::fs::write(&src_path, r#"{"foo":"bar"}"#).expect("write source");

        let guard =
            IsolatedConfigDir::with_sources(None, Some(&src_path)).expect("build should succeed");

        let written = std::fs::read_to_string(guard.path().join(".claude.json"))
            .expect(".claude.json should have been copied");
        assert_eq!(written, r#"{"foo":"bar"}"#);
    }

    #[test]
    fn skips_claude_json_silently_when_absent() {
        let src_dir = tempfile::tempdir().expect("scratch dir");
        let missing = src_dir.path().join("does-not-exist.json");

        let guard =
            IsolatedConfigDir::with_sources(None, Some(&missing)).expect("build should succeed");

        assert!(!guard.path().join(".claude.json").exists());
    }

    #[test]
    fn drop_removes_temp_dir() {
        let guard = IsolatedConfigDir::with_sources(None, None).expect("build should succeed");
        let path = guard.path().to_path_buf();
        assert!(path.exists());

        drop(guard);

        assert!(!path.exists());
    }

    #[test]
    fn mid_construction_failure_cleans_up_partial_dir() {
        let scratch = tempfile::tempdir().expect("scratch dir");
        // A directory (not a file) as the .claude.json source makes
        // `copy_if_present`'s `fs::read` fail with something other than
        // `NotFound`, forcing `build` to bail out after the temp dir (and its
        // already-written `.credentials.json`) were created.
        let bad_src = scratch.path().to_path_buf();

        let result = IsolatedConfigDir::build(
            Some(scratch.path()),
            Some(r#"{"claudeAiOauth":{"refreshToken":"secret"}}"#.to_string()),
            Some(&bad_src),
        );

        assert!(result.is_err());
        let remaining: Vec<_> = std::fs::read_dir(scratch.path())
            .expect("scratch dir should still exist")
            .collect();
        assert!(
            remaining.is_empty(),
            "partially-built temp dir should have been cleaned up, found: {remaining:?}"
        );
    }

    /// The regression this whole change exists for: a slow Keychain read must
    /// not park the runtime's worker thread.
    ///
    /// Runs on a single-threaded runtime — the strictest case, and the one that
    /// makes the failure unambiguous: with the read on the worker thread there
    /// is *no* other thread for the ticker task to run on, so it cannot advance
    /// at all while the read is in flight. Before the fix this asserts 0 ticks;
    /// with `spawn_blocking` the read moves off-worker and the ticker runs.
    #[tokio::test]
    async fn slow_keychain_read_does_not_block_the_runtime() {
        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ticker_ticks = std::sync::Arc::clone(&ticks);

        let ticker = tokio::spawn(async move {
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                ticker_ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        let guard = IsolatedConfigDir::new_async_with(
            || {
                std::thread::sleep(SLOW_KEYCHAIN_READ);
                None
            },
            read_file_credentials,
        )
        .await
        .expect("build should succeed");

        let observed = ticks.load(std::sync::atomic::Ordering::SeqCst);
        ticker.abort();
        drop(guard);

        assert!(
            observed > 0,
            "the async runtime made no progress while the keychain read ran \
             ({observed} ticks) — the blocking read is still on a worker thread"
        );
    }

    /// Concurrent isolated builds overlap rather than queueing end-to-end.
    ///
    /// Four slow reads dispatched at once should finish in roughly one read's
    /// time, not four, because `spawn_blocking`'s pool has many threads.
    /// `tokio::join!` polls all four before any of them can complete, so each
    /// reaches its `spawn_blocking` dispatch first — awaiting them in sequence
    /// instead would serialize them by construction and measure nothing. The
    /// bound is deliberately loose (half the fully-serialized time) so a loaded
    /// machine cannot flake it, while reads that actually serialize still fail.
    #[tokio::test]
    async fn concurrent_isolated_builds_overlap_instead_of_serializing() {
        const N: u32 = 4;
        fn slow_build() -> impl std::future::Future<Output = Result<IsolatedConfigDir>> {
            IsolatedConfigDir::new_async_with(
                || {
                    std::thread::sleep(SLOW_KEYCHAIN_READ);
                    None
                },
                read_file_credentials,
            )
        }

        let started = std::time::Instant::now();
        let (a, b, c, d) = tokio::join!(slow_build(), slow_build(), slow_build(), slow_build());
        let elapsed = started.elapsed();

        for guard in [&a, &b, &c, &d] {
            assert!(guard.is_ok(), "all concurrent builds should succeed");
        }
        drop((a, b, c, d));

        let serialized = SLOW_KEYCHAIN_READ * N;
        assert!(
            elapsed < serialized / 2,
            "{N} concurrent keychain reads took {elapsed:?}, close to the \
             fully-serialized {serialized:?} — they are not running on the \
             blocking pool concurrently"
        );
    }

    const EMPTY_TOKEN_BLOB: &str =
        r#"{"claudeAiOauth":{"accessToken":"","expiresAt":0,"subscriptionType":"max"}}"#;
    const VALID_BLOB: &str =
        r#"{"claudeAiOauth":{"accessToken":"sk-ant-real","expiresAt":9999999999999}}"#;

    #[test]
    fn usable_access_token_requires_a_non_empty_string() {
        assert!(has_usable_access_token(VALID_BLOB));
        assert!(!has_usable_access_token(EMPTY_TOKEN_BLOB));
        assert!(!has_usable_access_token(r#"{"claudeAiOauth":{}}"#));
        assert!(!has_usable_access_token("not json"));
    }

    #[test]
    fn no_source_at_all_returns_ok_none_without_retrying() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let keychain_calls = std::sync::Arc::clone(&calls);

        let result = read_credentials_with_retry(
            &move || {
                keychain_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                None
            },
            &|| None,
        );

        assert!(matches!(result, Ok(None)));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an absent source on both sides should not be retried"
        );
    }

    #[test]
    fn transiently_empty_token_self_heals_before_exhausting_attempts() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let keychain_calls = std::sync::Arc::clone(&calls);

        // Empty for the first two rounds, real on the third — well within
        // CREDENTIAL_READ_RETRY_ATTEMPTS.
        let result = read_credentials_with_retry(
            &move || {
                let n = keychain_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(if n < 2 {
                    EMPTY_TOKEN_BLOB.to_string()
                } else {
                    VALID_BLOB.to_string()
                })
            },
            &|| None,
        );

        assert_eq!(
            result.expect("should recover"),
            Some(VALID_BLOB.to_string())
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "should stop retrying as soon as a usable token appears"
        );
    }

    #[test]
    fn file_fallback_wins_immediately_when_keychain_is_invalid() {
        let result = read_credentials_with_retry(&|| Some(EMPTY_TOKEN_BLOB.to_string()), &|| {
            Some(VALID_BLOB.to_string())
        });

        assert_eq!(
            result.expect("should use the file"),
            Some(VALID_BLOB.to_string())
        );
    }

    #[test]
    fn persistently_empty_token_errors_after_exhausting_attempts() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let keychain_calls = std::sync::Arc::clone(&calls);

        let result = read_credentials_with_retry(
            &move || {
                keychain_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(EMPTY_TOKEN_BLOB.to_string())
            },
            &|| None,
        );

        assert!(
            result.is_err(),
            "should surface a diagnosable error, not silently proceed"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            CREDENTIAL_READ_RETRY_ATTEMPTS as usize,
            "should have made exactly the bounded number of attempts"
        );
    }

    #[tokio::test]
    async fn build_recovers_through_new_with_when_source_self_heals() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let keychain_calls = std::sync::Arc::clone(&calls);

        let guard = IsolatedConfigDir::new_async_with(
            move || {
                let n = keychain_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(if n == 0 {
                    EMPTY_TOKEN_BLOB.to_string()
                } else {
                    VALID_BLOB.to_string()
                })
            },
            || None,
        )
        .await
        .expect("build should recover once the source self-heals");

        let written = std::fs::read_to_string(guard.path().join(".credentials.json"))
            .expect("credentials file should exist");
        assert!(written.contains("sk-ant-real"));
    }

    #[tokio::test]
    async fn build_fails_clearly_when_source_never_recovers() {
        let result =
            IsolatedConfigDir::new_async_with(|| Some(EMPTY_TOKEN_BLOB.to_string()), || None).await;

        assert!(
            result.is_err(),
            "a persistently-empty credential source should fail the build, not write junk"
        );
    }
}
