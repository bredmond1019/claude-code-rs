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

use tempfile::TempDir;

use crate::error::{Error, Result};

/// macOS Keychain service name under which the `claude` CLI stores OAuth
/// credentials when `CLAUDE_CONFIG_DIR` is unset.
const KEYCHAIN_SERVICE_NAME: &str = "Claude Code-credentials";

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
    /// # Errors
    /// Returns [`Error::Isolation`] if the temp directory cannot be created or
    /// a source file exists but cannot be copied.
    pub fn new() -> Result<Self> {
        let creds_json = read_keychain_credentials().or_else(read_file_credentials);
        let claude_json_src = home_dir().map(|home| home.join(".claude.json"));
        Self::build(None, creds_json, claude_json_src.as_deref())
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
