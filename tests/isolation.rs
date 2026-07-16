//! Integration tests for the credential isolation seam (`CC.1.B`).
//!
//! Exercises the public API only: [`claude_code_rs::IsolatedConfigDir`] and
//! [`claude_code_rs::execute`] with `Config { isolated: true, .. }`. All
//! credential sourcing here is injected via `with_sources` — none of these
//! tests touch the real macOS Keychain or `~/.claude/`.

use claude_code_rs::{Config, IsolatedConfigDir};

/// Building the isolated dir from a canned credentials JSON and a copied
/// `.claude.json` source produces the expected `~/.claude/`-shaped layout:
/// a redacted `.credentials.json` (mode 0600 on unix, `refreshToken` gone,
/// other fields intact) plus a byte-identical `.claude.json`.
#[test]
fn temp_dir_layout_is_redacted_credentials_plus_claude_json() {
    let creds_json = r#"{"claudeAiOauth":{"accessToken":"abc123","refreshToken":"super-secret","expiresAt":999}}"#;

    let claude_json_dir = tempfile::tempdir().expect("scratch dir for .claude.json source");
    let claude_json_src = claude_json_dir.path().join(".claude.json");
    std::fs::write(&claude_json_src, r#"{"projects":{}}"#).expect("write .claude.json source");

    let guard =
        IsolatedConfigDir::with_sources(Some(creds_json.to_string()), Some(&claude_json_src))
            .expect("isolated config dir should build");

    // .credentials.json: redacted, permissions locked down.
    let creds_path = guard.path().join(".credentials.json");
    assert!(creds_path.exists(), ".credentials.json should exist");

    let written_creds = std::fs::read_to_string(&creds_path).expect("read .credentials.json");
    let value: serde_json::Value =
        serde_json::from_str(&written_creds).expect("written creds should be valid JSON");
    assert!(
        value["claudeAiOauth"].get("refreshToken").is_none(),
        "refreshToken must be deleted from the written credentials"
    );
    assert_eq!(value["claudeAiOauth"]["accessToken"], "abc123");
    assert_eq!(value["claudeAiOauth"]["expiresAt"], 999);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&creds_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, ".credentials.json must be mode 0600 on unix");
    }

    // .claude.json: copied verbatim.
    let claude_json_dst = guard.path().join(".claude.json");
    assert!(
        claude_json_dst.exists(),
        ".claude.json should be copied when a source exists"
    );
    let written_claude_json =
        std::fs::read_to_string(&claude_json_dst).expect("read copied .claude.json");
    assert_eq!(written_claude_json, r#"{"projects":{}}"#);
}

/// When no `.claude.json` source is given (or it doesn't exist), the temp dir
/// still builds successfully and simply omits the file.
#[test]
fn temp_dir_omits_claude_json_when_source_absent() {
    let guard = IsolatedConfigDir::with_sources(
        Some(r#"{"claudeAiOauth":{"refreshToken":"x"}}"#.to_string()),
        None,
    )
    .expect("isolated config dir should build without a .claude.json source");

    assert!(guard.path().join(".credentials.json").exists());
    assert!(!guard.path().join(".claude.json").exists());
}

/// Dropping the guard removes the whole temp directory tree.
#[test]
fn dropping_guard_removes_temp_directory() {
    let guard = IsolatedConfigDir::with_sources(
        Some(r#"{"claudeAiOauth":{"refreshToken":"x"}}"#.to_string()),
        None,
    )
    .expect("isolated config dir should build");

    let path = guard.path().to_path_buf();
    assert!(path.exists(), "temp dir should exist while guard is alive");

    drop(guard);

    assert!(
        !path.exists(),
        "temp dir should be removed once the guard is dropped"
    );
}

/// Live smoke test: runs an isolated `execute()` call against a trivial
/// prompt. This is the manual proof (per the block's acceptance criteria)
/// that a subprocess spawned under `Config { isolated: true, .. }` does not
/// disturb — and can safely run concurrently with — a normal interactive
/// `claude` session, since it operates against a throwaway, redacted
/// `CLAUDE_CONFIG_DIR` rather than the real `~/.claude/`.
///
/// Ignored so the gated `cargo test` run stays hermetic and fast; run
/// manually with `cargo test -- --ignored` when the `claude` CLI and a real
/// interactive session are available.
#[tokio::test]
#[ignore]
async fn live_isolated_execute_does_not_disturb_interactive_session() {
    let isolated_config = Config {
        isolated: true,
        ..Config::default()
    };

    // Run the isolated call concurrently with a normal (non-isolated) call to
    // approximate an interactive session happening at the same time. Both
    // must succeed and return a populated Outcome.
    let interactive_config = Config::default();

    let (isolated_result, interactive_result) = tokio::join!(
        claude_code_rs::execute(&isolated_config, "Say hello in one word."),
        claude_code_rs::execute(&interactive_config, "Say hi in one word."),
    );

    let isolated_outcome = isolated_result.expect("isolated execute should succeed");
    let interactive_outcome = interactive_result.expect("interactive execute should succeed");

    assert!(!isolated_outcome.text.is_empty());
    assert!(isolated_outcome.primary_model().is_some());
    assert!(isolated_outcome.cost_usd >= 0.0);
    assert!(!interactive_outcome.text.is_empty());
    assert!(interactive_outcome.primary_model().is_some());
    assert!(interactive_outcome.cost_usd >= 0.0);
}
