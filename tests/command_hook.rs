//! Rendering and ordering of `Config::pre_tool_use_hooks` in the merged `--settings` object.

use std::path::{Path, PathBuf};

use claude_sdk_rs::{Config, Endpoint, Error};
use serde_json::Value;

const FIXTURE_COMMAND: &str = "/fixture/only/deny-hook.sh";

fn settings(config: &Config) -> Option<Value> {
    let args = config.build_args("hi");
    let idx = args.iter().position(|a| a == "--settings")?;
    Some(serde_json::from_str(&args[idx + 1]).expect("--settings is JSON"))
}

fn pre_tool_use(config: &Config) -> Vec<Value> {
    settings(config).map_or_else(Vec::new, |s| {
        s["hooks"]["PreToolUse"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    })
}

fn deny() -> Vec<String> {
    vec!["*/.env".to_string()]
}

#[test]
fn no_hooks_leaves_existing_outputs_unchanged() {
    assert!(settings(&Config::default()).is_none());
    let deny_only = Config {
        denied_path_patterns: deny(),
        ..Default::default()
    };
    assert_eq!(pre_tool_use(&deny_only).len(), 1);
    let confine_only = Config {
        confine_to: Some(PathBuf::from("/tmp/root")),
        ..Default::default()
    };
    assert_eq!(pre_tool_use(&confine_only).len(), 1);
    let all = Config {
        denied_path_patterns: deny(),
        confine_to: Some(PathBuf::from("/tmp/root")),
        endpoint: Some(Endpoint::new("http://localhost:11434", "m")),
        ..Default::default()
    };
    assert_eq!(pre_tool_use(&all).len(), 2);
    assert!(settings(&all).unwrap().get("env").is_some());
}

#[test]
fn hooks_follow_deny_and_confine_in_order_verbatim() {
    let config = Config {
        denied_path_patterns: deny(),
        confine_to: Some(PathBuf::from("/tmp/root")),
        ..Default::default()
    }
    .with_pre_tool_use_hook("Bash", FIXTURE_COMMAND)
    .unwrap()
    .with_pre_tool_use_hook("Edit|Write", "second-hook")
    .unwrap();
    let entries = pre_tool_use(&config);
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["matcher"], "Read|Grep|Glob");
    assert!(entries[1]["matcher"].as_str().unwrap().contains("Edit"));
    assert_eq!(entries[2]["matcher"], "Bash");
    assert_eq!(entries[2]["hooks"][0]["type"], "command");
    assert_eq!(entries[2]["hooks"][0]["command"], FIXTURE_COMMAND);
    assert_eq!(entries[3]["matcher"], "Edit|Write");
    assert_eq!(entries[3]["hooks"][0]["command"], "second-hook");
}

#[test]
fn hook_alone_still_emits_settings() {
    let config = Config::default()
        .with_pre_tool_use_hook("Bash", FIXTURE_COMMAND)
        .unwrap();
    assert_eq!(pre_tool_use(&config).len(), 1);
}

#[test]
fn blank_matcher_or_command_is_refused() {
    assert!(matches!(
        Config::default().with_pre_tool_use_hook("", "x"),
        Err(Error::InvalidCommandHook { field: "matcher" })
    ));
    assert!(matches!(
        Config::default().with_pre_tool_use_hook("Bash", ""),
        Err(Error::InvalidCommandHook { field: "command" })
    ));
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rs_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn crate_source_carries_no_copy_of_the_fixture_command() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    // Positive control: the walk must see real source.
    assert!(files.iter().any(|f| f.ends_with("settings.rs")));
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(!text.contains(FIXTURE_COMMAND), "{} embeds it", f.display());
    }
}
