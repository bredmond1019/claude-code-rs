//! Live checks for `Config::endpoint` + `Config::confine_to` against a real Anthropic-compatible
//! server (Ollama). All `#[ignore]`d; run manually with the real `claude` CLI and a signed-in
//! Ollama daemon, ideally through a loopback logging proxy so "no call reached Anthropic" is
//! observed rather than assumed:
//!
//! ```text
//! CCRS_LIVE_BASE_URL=http://127.0.0.1:11434 CCRS_LIVE_MODEL=gpt-oss:120b-cloud \
//!   cargo test --test endpoint_live -- --ignored --test-threads=1
//! ```
//!
//! No Anthropic credential is involved: `isolated` with an endpoint writes none, and
//! `ANTHROPIC_API_KEY` is removed from the child.

use std::time::Duration;

use claude_sdk_rs::{Config, Endpoint, Outcome};

const OUTSIDE_FILE: &str = "/etc/hosts";

fn endpoint() -> Endpoint {
    let base = std::env::var("CCRS_LIVE_BASE_URL").unwrap_or("http://127.0.0.1:11434".into());
    let model = std::env::var("CCRS_LIVE_MODEL").unwrap_or("gpt-oss:120b-cloud".into());
    Endpoint::new(base, model)
}

fn marker() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("ROOT-MARKER-{nanos}")
}

fn base_config(cwd: &std::path::Path) -> Config {
    Config {
        endpoint: Some(endpoint()),
        isolated: true,
        cwd: Some(cwd.to_path_buf()),
        // So a refusal can only come from the confine hook, not the CLI's own prompt policy.
        dangerously_skip_permissions: true,
        max_turns: Some(6),
        timeout: Some(Duration::from_secs(240)),
        ..Config::default()
    }
}

fn prompt(file: &std::path::Path) -> String {
    format!(
        "Use the Read tool on {} and then use the Read tool on {OUTSIDE_FILE}. \
         Then report the first line of each file verbatim, or say it was refused.",
        file.display()
    )
}

async fn run(config: &Config, prompt: &str) -> Outcome {
    let outcome = claude_sdk_rs::execute(config, prompt)
        .await
        .expect("execute should succeed");
    eprintln!(
        "text={:?}\nmodel_usage={:?}\ncost_usd={}\nturns={:?}\ndenials={:?}",
        outcome.text,
        outcome.model_usage.keys().collect::<Vec<_>>(),
        outcome.cost_usd,
        outcome.num_turns,
        outcome.permission_denials
    );
    outcome
}

fn reads_of(outcome: &Outcome, path: &str) -> usize {
    outcome
        .permission_denials
        .iter()
        .filter(|d| d.tool_name == "Read" && d.tool_input["file_path"] == path)
        .count()
}

/// The in-root read works, the `/etc/hosts` read is refused by the confine hook and recorded on
/// `permission_denials`, and every model the CLI billed is the endpoint's (no `claude-*`).
#[tokio::test]
#[ignore]
async fn live_confined_endpoint_reads_root_file_and_is_refused_outside() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let marker = marker();
    let file = root_path.join("notes.txt");
    std::fs::write(&file, format!("{marker}\n")).unwrap();

    let config = Config {
        confine_to: Some(root_path.clone()),
        ..base_config(&root_path)
    };
    let outcome = run(&config, &prompt(&file)).await;

    assert!(outcome.text.contains(&marker), "in-root read must work");
    assert!(
        reads_of(&outcome, OUTSIDE_FILE) >= 1,
        "the /etc/hosts read must be on permission_denials"
    );
    assert_eq!(reads_of(&outcome, &file.to_string_lossy()), 0);
    assert!(
        outcome.model_usage.keys().all(|m| !m.starts_with("claude")),
        "no Anthropic model billed: {:?}",
        outcome.model_usage.keys()
    );
}

/// Positive control: the same run with no `confine_to` reads `/etc/hosts`, so the test above's
/// denial is the hook's doing, not a model that never tried or a CLI default.
#[tokio::test]
#[ignore]
async fn live_unconfined_control_reads_outside_file() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let file = root_path.join("notes.txt");
    std::fs::write(&file, format!("{}\n", marker())).unwrap();

    let outcome = run(&base_config(&root_path), &prompt(&file)).await;

    assert!(outcome.permission_denials.is_empty());
    let first_line = std::fs::read_to_string(OUTSIDE_FILE)
        .ok()
        .and_then(|s| s.lines().next().map(str::to_string))
        .unwrap_or_default();
    assert!(
        !first_line.is_empty() && outcome.text.contains(&first_line),
        "without confinement the first line of {OUTSIDE_FILE} ({first_line:?}) should be reported"
    );
}

/// A project `settings.json` in `cwd` whose `env` block re-points the base URL at a dead port
/// cannot override the endpoint: the inline `--settings` env outranks project settings.
#[tokio::test]
#[ignore]
async fn live_project_settings_cannot_repoint_the_endpoint() {
    let cwd = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(cwd.path().join(".claude")).unwrap();
    std::fs::write(
        cwd.path().join(".claude/settings.json"),
        r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9","ANTHROPIC_MODEL":"claude-opus-4-1"}}"#,
    )
    .unwrap();
    let config = Config {
        max_turns: Some(1),
        ..base_config(cwd.path())
    };

    let outcome = run(&config, "Reply with just the word ok.").await;

    assert!(!outcome.text.is_empty());
    assert!(outcome.model_usage.keys().all(|m| !m.starts_with("claude")));
}
