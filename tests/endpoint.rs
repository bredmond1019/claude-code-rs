//! `Config::endpoint` — argv, the `--settings` env block, the child's environment, and the
//! isolation/heal interplay. The child is a fake `/bin/sh` script named by `CLAUDE_BINARY`, so
//! nothing here reaches a real CLI or network.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use claude_sdk_rs::endpoint::{
    AUTH_TOKEN_ENV_VAR, BASE_URL_ENV_VAR, DEFAULT_PLACEHOLDER_AUTH_TOKEN, FIXED_ENV_VARS,
    MODEL_ENV_VARS, REMOVED_ENV_VARS,
};
use claude_sdk_rs::{Config, Endpoint, Error};

const BASE_URL: &str = "http://127.0.0.1:11434";
const TAG: &str = "gpt-oss:120b-cloud";

/// `CLAUDE_BINARY` is process-global; every test that sets it holds this lock.
static BINARY_LOCK: Mutex<()> = Mutex::new(());

fn endpoint() -> Endpoint {
    Endpoint::new(BASE_URL, TAG)
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter(|w| w[0] == flag)
        .map(|w| w[1].as_str())
        .collect()
}

fn settings_of(config: &Config) -> serde_json::Value {
    let args = config.build_args("hi");
    let values = flag_value(&args, "--settings");
    assert_eq!(values.len(), 1, "exactly one --settings: {args:?}");
    serde_json::from_str(values[0]).expect("--settings is JSON")
}

#[test]
fn default_config_has_no_endpoint_and_argv_matches_pre_endpoint_golden() {
    let golden: Vec<Vec<String>> = include_str!("fixtures/argv-golden-pre-endpoint.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).expect("golden line"))
        .collect();

    assert!(Config::default().endpoint.is_none());
    assert!(Config::default().confine_to.is_none());

    let deny_only = Config {
        denied_path_patterns: vec!["*/.env".into(), "a\"b".into()],
        setting_sources: Some(vec![]),
        ..Config::default()
    };
    assert_eq!(
        deny_only.build_args("hi"),
        golden[0],
        "deny-only argv byte-identical"
    );
    assert_eq!(
        Config::default().build_args("hi"),
        golden[1],
        "default argv byte-identical"
    );
}

#[test]
fn endpoint_passes_its_model_and_wins_over_config_model() {
    let config = Config {
        model: Some("claude-opus-4".into()),
        endpoint: Some(endpoint()),
        ..Config::default()
    };
    assert_eq!(flag_value(&config.build_args("hi"), "--model"), vec![TAG]);
}

#[test]
fn endpoint_settings_env_pins_every_variable_and_blanks_the_api_key() {
    let config = Config {
        endpoint: Some(endpoint()),
        ..Config::default()
    };
    let env = &settings_of(&config)["env"];
    assert_eq!(env[BASE_URL_ENV_VAR], BASE_URL);
    assert_eq!(env[AUTH_TOKEN_ENV_VAR], DEFAULT_PLACEHOLDER_AUTH_TOKEN);
    for name in MODEL_ENV_VARS {
        assert_eq!(env[*name], TAG, "{name}");
    }
    for (name, value) in FIXED_ENV_VARS {
        assert_eq!(env[*name], *value, "{name}");
    }
    for name in REMOVED_ENV_VARS {
        assert_eq!(env[*name], "", "{name}");
    }
    assert!(settings_of(&config).get("hooks").is_none());
}

#[test]
fn endpoint_composes_with_deny_list_in_one_settings_object() {
    let config = Config {
        endpoint: Some(endpoint()),
        denied_path_patterns: vec!["*/.env".into()],
        ..Config::default()
    };
    let settings = settings_of(&config);
    assert!(settings["env"].is_object());
    assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
}

/// A fake CLI that records its run in `counter` and reports the child's ANTHROPIC_*/CLAUDE_*
/// env plus the isolated config dir's listing in the result text.
fn write_fake_binary(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn with_binary<T>(binary: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = BINARY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: serialized by BINARY_LOCK; no other thread in this test binary reads it unlocked.
    unsafe { std::env::set_var("CLAUDE_BINARY", binary) };
    let out = f();
    unsafe { std::env::remove_var("CLAUDE_BINARY") };
    out
}

fn run(config: &Config) -> claude_sdk_rs::Result<claude_sdk_rs::Outcome> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(claude_sdk_rs::execute(config, "hi"))
}

const REPORT_ENV: &str = r#"vars=$(env | grep -E '^(ANTHROPIC|CLAUDE_CODE)_' | sort | tr '\n' ';')
files=$( [ -n "$CLAUDE_CONFIG_DIR" ] && ls -A "$CLAUDE_CONFIG_DIR" | tr '\n' ',' )
printf '{"total_cost_usd":0.0,"usage":{},"is_error":false,"result":"%s|%s"}' "$vars" "$files"
"#;

fn reported(outcome: &claude_sdk_rs::Outcome) -> (BTreeMap<String, String>, Vec<String>) {
    let (vars, files) = outcome.text.split_once('|').expect("vars|files");
    let vars = vars
        .split(';')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let files = files
        .split(',')
        .filter(|f| !f.is_empty())
        .map(String::from)
        .collect();
    (vars, files)
}

#[test]
fn child_env_carries_endpoint_vars_and_never_the_api_key() {
    let dir = tempfile::tempdir().unwrap();
    let binary = write_fake_binary(dir.path(), REPORT_ENV);
    let config = Config {
        endpoint: Some(endpoint()),
        // A caller override must not re-add the key or re-point the model.
        env: vec![
            ("ANTHROPIC_API_KEY".into(), "sk-test-not-a-real-key".into()),
            ("ANTHROPIC_MODEL".into(), "claude-opus-4".into()),
        ],
        ..Config::default()
    };
    let (vars, _) = reported(&with_binary(&binary, || run(&config)).expect("fake run"));

    assert_eq!(vars[BASE_URL_ENV_VAR], BASE_URL);
    assert_eq!(vars[AUTH_TOKEN_ENV_VAR], DEFAULT_PLACEHOLDER_AUTH_TOKEN);
    for name in MODEL_ENV_VARS {
        assert_eq!(vars.get(*name).map(String::as_str), Some(TAG), "{name}");
    }
    for (name, value) in FIXED_ENV_VARS {
        assert_eq!(vars.get(*name).map(String::as_str), Some(*value), "{name}");
    }
    for name in REMOVED_ENV_VARS {
        assert!(!vars.contains_key(*name), "{name} must be removed");
    }
}

#[test]
fn isolated_endpoint_dir_holds_no_credentials_and_no_settings() {
    let dir = tempfile::tempdir().unwrap();
    let binary = write_fake_binary(dir.path(), REPORT_ENV);
    let config = Config {
        endpoint: Some(endpoint()),
        isolated: true,
        ..Config::default()
    };
    let (_, files) = reported(&with_binary(&binary, || run(&config)).expect("fake run"));
    assert!(
        !files.contains(&".credentials.json".to_string()),
        "{files:?}"
    );
    assert!(!files.contains(&"settings.json".to_string()), "{files:?}");
}

#[test]
fn credential_free_isolated_dir_has_no_credentials_file() {
    let guard = claude_sdk_rs::IsolatedConfigDir::without_credentials().expect("build");
    assert!(!guard.path().join(".credentials.json").exists());
    assert!(!guard.path().join("settings.json").exists());
}

/// A fake CLI that always fails with the isolated-OAuth-expired shape and counts its runs.
fn oauth_expired_binary(dir: &Path) -> (PathBuf, PathBuf) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cli-error-oauth-expired-2.1.270.json");
    let counter = dir.join("runs");
    let body = format!(
        "echo run >> '{}'\ncat '{}'\nexit 1\n",
        counter.display(),
        fixture.display()
    );
    (write_fake_binary(dir, &body), counter)
}

fn runs(counter: &Path) -> usize {
    std::fs::read_to_string(counter).map_or(0, |s| s.lines().count())
}

#[test]
fn heal_retry_is_skipped_with_an_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let (binary, counter) = oauth_expired_binary(dir.path());
    let config = Config {
        endpoint: Some(endpoint()),
        isolated: true,
        heal_isolated_auth_on_expiry: true,
        ..Config::default()
    };
    let err = with_binary(&binary, || run(&config)).expect_err("fixture is an error");
    assert!(
        matches!(
            err,
            Error::Api {
                status: Some(401),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(runs(&counter), 1, "no heal spawn, no retry");
}
