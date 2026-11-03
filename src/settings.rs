//! Composes the single inline `--settings` JSON object a call passes to the CLI.
//!
//! Several features contribute to it — the deny-list hook, the allow-root hook and the endpoint's
//! `env` block — but the CLI takes one `--settings` value, so they are merged here rather than
//! each emitting its own flag. With only `denied_path_patterns` set the output is byte-identical
//! to the deny-only object this crate emitted before the other contributors existed.

use serde_json::{json, Map, Value};

use crate::config::Config;

/// The merged settings object, or `None` when no contributor is active (the flag is omitted).
pub(crate) fn cli_settings(config: &Config) -> Option<Value> {
    let mut pre_tool_use = Vec::new();
    if !config.denied_path_patterns.is_empty() {
        pre_tool_use.push(crate::permission_hook::hook_entry(
            &config.denied_path_patterns,
        ));
    }
    if let Some(root) = &config.confine_to {
        let root = crate::confine::resolve_root(root, config.cwd.as_deref());
        pre_tool_use.push(crate::confine::hook_entry(&root));
    }

    let mut settings = Map::new();
    if let Some(endpoint) = &config.endpoint {
        settings.insert("env".to_string(), endpoint.settings_env());
    }
    if !pre_tool_use.is_empty() {
        settings.insert("hooks".to_string(), json!({ "PreToolUse": pre_tool_use }));
    }
    (!settings.is_empty()).then_some(Value::Object(settings))
}
