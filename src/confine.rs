//! Allow-root confinement (`Config::confine_to`): a fail-closed `PreToolUse` hook that refuses any
//! file tool whose path inputs resolve outside one root directory, plus the tools that reach
//! beyond the filesystem entirely, which are removed with `--disallowedTools`.
//!
//! The hook complements `permission_hook`'s deny-list: both are separate `PreToolUse` entries in
//! one `--settings` object, and the CLI refuses a call when either blocks it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// File tools whose path input is required, and the key it arrives under.
pub const REQUIRED_PATH_TOOLS: &[(&str, &str)] = &[
    ("Read", "file_path"),
    ("Edit", "file_path"),
    ("Write", "file_path"),
    ("MultiEdit", "file_path"),
    ("NotebookEdit", "notebook_path"),
];

/// Search tools whose path input is optional; when absent the CLI searches the session `cwd`,
/// which the hook then requires to be inside the root.
pub const OPTIONAL_PATH_TOOLS: &[(&str, &str)] = &[("Grep", "path"), ("Glob", "path")];

/// Tools with no path to check — network access and subagents — removed via `--disallowedTools`
/// whenever `confine_to` is set. `Task` is the pre-2.1 name of `Agent`; both are listed.
pub const CONFINE_DISALLOWED_TOOLS: &[&str] = &["WebFetch", "WebSearch", "Task", "Agent"];

/// The `reason` the hook reports when it blocks a call.
pub const CONFINE_DENY_REASON: &str = "denied: path outside confine_to root";

/// The hook body. `__CONFIG__` is replaced by a JSON object carrying the root, the tool tables and
/// the reason. Every failure path — bad stdin, a non-string path, an exception — blocks.
const HOOK_SOURCE: &str = r#"import sys,json,os
C=json.loads(__CONFIG__)
def inside(base,p):
    r=os.path.realpath(os.path.join(base,p))
    return r==ROOT or r.startswith(ROOT.rstrip(os.sep)+os.sep)
def allowed():
    global ROOT
    ROOT=os.path.realpath(C["root"])
    d=json.load(sys.stdin)
    t=d.get("tool_name");i=d.get("tool_input");c=d.get("cwd")
    if not isinstance(i,dict) or not isinstance(c,str) or not os.path.isabs(c):
        return False
    keys=set(C["required"].values())|set(C["optional"].values())
    for k in keys:
        if i.get(k) is not None:
            v=i[k]
            if not isinstance(v,str) or v=="" or not inside(c,v):
                return False
    if t in C["required"] and not i.get(C["required"][t]):
        return False
    if t in C["optional"] and not i.get(C["optional"][t]) and not inside(c,"."):
        return False
    if t=="Glob":
        pat=i.get("pattern")
        if not isinstance(pat,str) or ".." in pat.replace(os.sep,"/").split("/"):
            return False
        full=os.path.join(i.get("path") or c,pat)
        cut=min([full.find(m) for m in "*?[{" if m in full] or [len(full)])
        if not inside(c,os.path.dirname(full[:cut]) or os.sep):
            return False
    return True
try:
    ok=allowed()
except Exception:
    ok=False
print("{}" if ok else json.dumps({"decision":"block","reason":C["reason"]}))
"#;

/// The `PreToolUse` matcher: every tool in both path tables, `|`-joined.
#[must_use]
pub fn matcher() -> String {
    REQUIRED_PATH_TOOLS
        .iter()
        .chain(OPTIONAL_PATH_TOOLS)
        .map(|(tool, _)| *tool)
        .collect::<Vec<_>>()
        .join("|")
}

/// `root` made absolute: relative roots resolve against `cwd` when set, else against this
/// process's working directory (which the spawned CLI inherits).
#[must_use]
pub fn resolve_root(root: &Path, cwd: Option<&Path>) -> PathBuf {
    if root.is_absolute() {
        return root.to_path_buf();
    }
    match cwd {
        Some(dir) => dir.join(root),
        None => std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf()),
    }
}

/// The shell command the hook runs: `python3 -c '<HOOK_SOURCE>'` with the config embedded and
/// every single quote shell-escaped, so any root path survives quoting.
#[must_use]
pub fn hook_command(root: &Path) -> String {
    let table = |pairs: &[(&str, &str)]| -> Value {
        pairs
            .iter()
            .map(|(tool, key)| ((*tool).to_string(), Value::String((*key).to_string())))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    let config = json!({
        "root": root.to_string_lossy(),
        "required": table(REQUIRED_PATH_TOOLS),
        "optional": table(OPTIONAL_PATH_TOOLS),
        "reason": CONFINE_DENY_REASON,
    });
    // A JSON string literal is a valid Python string literal for json.loads to parse.
    let config_literal = Value::String(config.to_string()).to_string();
    let source = HOOK_SOURCE.replace("__CONFIG__", &config_literal);
    format!("python3 -c '{}'", source.replace('\'', r"'\''"))
}

/// One `PreToolUse` entry confining the file tools to `root`.
pub(crate) fn hook_entry(root: &Path) -> Value {
    json!({
        "matcher": matcher(),
        "hooks": [{ "type": "command", "command": hook_command(root) }]
    })
}
