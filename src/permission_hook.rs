//! Builder for the `--settings` JSON that enforces path-scoped tool denial.
//!
//! The real `claude` CLI runs a `PreToolUse` hook command before a tool call and honours a
//! `{"decision":"block","reason":...}` reply on its stdout by refusing the call (verified live
//! against 2.1.273; the hook's stdin shape is captured in
//! `tests/fixtures/hook-pretooluse-stdin-2.1.273.json`). The refusal is recorded on the result
//! envelope's `permission_denials`, surfaced as `Outcome::permission_denials`.

use serde_json::{json, Value};

/// The tools the hook guards: the read-oriented ones that take a path in `tool_input.file_path`.
const MATCHER: &str = "Read|Grep|Glob";

/// The `reason` the hook reports when it blocks a call.
const DENY_REASON: &str = "denied: secret-shaped path";

/// Build the `--settings` JSON value embedding `patterns` into a `PreToolUse` deny hook.
///
/// The hook command is a `python3 -c` one-liner that reads the hook's stdin JSON, fnmatch-tests
/// `tool_input.file_path` against `patterns`, and prints a block decision on a match or `{}`
/// otherwise. Each pattern is rendered with Rust's `{:?}` into a Python string literal, which
/// escapes quotes and backslashes compatibly. An empty `patterns` yields a well-formed object whose
/// hook never blocks; the caller decides whether to emit it at all.
// Consumed by `Config::build_args` in the next task; unused in non-test builds until then.
#[allow(dead_code)]
pub(crate) fn settings_json(patterns: &[String]) -> Value {
    let literals = patterns
        .iter()
        .map(|p| format!("{p:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let command = format!(
        "python3 -c 'import sys,json,fnmatch; d=json.load(sys.stdin); \
         p=(d.get(\"tool_input\") or {{}}).get(\"file_path\") or \"\"; \
         pats=[{literals}]; \
         print(json.dumps({{\"decision\":\"block\",\"reason\":\"{DENY_REASON}\"}}) \
         if any(fnmatch.fnmatch(p,x) for x in pats) else \"{{}}\")'"
    );
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": MATCHER,
                "hooks": [{ "type": "command", "command": command }]
            }]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_pattern_and_matcher() {
        let v = settings_json(&["*/.env".to_string()]);
        let entry = &v["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "Read|Grep|Glob");
        assert_eq!(entry["hooks"][0]["type"], "command");
        let command = entry["hooks"][0]["command"].as_str().expect("string");
        assert!(command.contains("*/.env"), "command: {command}");
        assert!(command.contains("block"));
    }

    #[test]
    fn empty_patterns_is_well_formed() {
        let v = settings_json(&[]);
        let entry = &v["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "Read|Grep|Glob");
        assert!(entry["hooks"][0]["command"].is_string());
    }

    #[test]
    fn quotes_in_patterns_are_escaped() {
        let v = settings_json(&["a\"b".to_string()]);
        let command = v["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("string");
        assert!(command.contains(r#""a\"b""#), "command: {command}");
    }
}
