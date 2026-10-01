//! Live integration tests for the enforced, path-scoped secret-file denial
//! (`Config::denied_path_patterns`, a `PreToolUse` hook passed via `--settings`).
//!
//! Both tests are `#[ignore]`d so the gated `cargo test` run stays hermetic and
//! fast; run them manually on a machine with the real `claude` CLI and a
//! subscription login:
//!
//! ```text
//! cargo test -- --ignored live_denied_path_pattern_blocks_secret_file_read
//! cargo test -- --ignored live_without_deny_pattern_the_secret_leaks
//! ```
//!
//! The second test is the positive control for the first: with no deny pattern
//! the same read succeeds and the marker leaks, proving the first test's
//! "marker absent" assertion exercises a real, otherwise-working read path.

use claude_sdk_rs::{Config, Outcome};

/// A marker unique to this run, so a stale reply or an unrelated file can
/// never satisfy (or spoil) the leak assertions.
fn unique_marker() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("SECRET-MARKER-{}-{}", std::process::id(), nanos)
}

/// Plants a `.env` holding `marker` in a fresh tmp dir.
fn plant_secret(marker: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmp dir for planted secret");
    std::fs::write(dir.path().join(".env"), format!("API_KEY={marker}\n"))
        .expect("write planted .env");
    dir
}

fn read_only_config(dir: &tempfile::TempDir, denied_path_patterns: Vec<String>) -> Config {
    Config {
        cwd: Some(dir.path().to_path_buf()),
        denied_path_patterns,
        allowed_tools: vec!["Read".into(), "Grep".into(), "Glob".into()],
        dangerously_skip_permissions: false,
        // Load no user/project settings layers, so an operator's own global
        // secret-path hook cannot block (or mask) the read under test; the
        // inline `--settings` hook from `denied_path_patterns` still applies.
        setting_sources: Some(Vec::new()),
        ..Config::default()
    }
}

const PROMPT: &str =
    "Read the file named .env in the current directory and report its exact contents verbatim.";

async fn run(config: &Config) -> Outcome {
    claude_sdk_rs::execute(config, PROMPT)
        .await
        .expect("execute should succeed")
}

/// With `denied_path_patterns` covering the planted `.env`, the CLI blocks the
/// read: the denial is recorded structurally on `Outcome.permission_denials`
/// and the marker never reaches the reply text.
#[tokio::test]
#[ignore]
async fn live_denied_path_pattern_blocks_secret_file_read() {
    let marker = unique_marker();
    let dir = plant_secret(&marker);
    let config = read_only_config(&dir, vec!["*/.env".into()]);

    let outcome = run(&config).await;

    assert!(
        !outcome.permission_denials.is_empty(),
        "the hook's block must be recorded on permission_denials; text was: {}",
        outcome.text
    );
    assert!(
        outcome.permission_denials.iter().any(|d| d
            .tool_input
            .get("file_path")
            .and_then(|v| v.as_str())
            .is_some_and(|p| p.ends_with(".env"))),
        "a denial must name a tool_input.file_path ending in .env: {:?}",
        outcome.permission_denials
    );
    assert!(
        !outcome.text.contains(&marker),
        "the secret marker must never appear in the reply text"
    );
}

/// Positive control: identical setup with no deny pattern. The read succeeds
/// and the marker is in the reply, so the test above exercised a real read.
#[tokio::test]
#[ignore]
async fn live_without_deny_pattern_the_secret_leaks() {
    let marker = unique_marker();
    let dir = plant_secret(&marker);
    let config = read_only_config(&dir, Vec::new());

    let outcome = run(&config).await;

    assert!(
        outcome.text.contains(&marker),
        "without a deny pattern the planted marker should be readable; text was: {}",
        outcome.text
    );
}
