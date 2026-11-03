//! `Config::confine_to` — the allow-root `PreToolUse` hook's command, taken from `build_args`'
//! `--settings` exactly as the CLI would receive it, run under `sh -c` against hook-stdin
//! fixtures for every guarded tool shape. The reference shape is the captured
//! `tests/fixtures/hook-pretooluse-stdin-2.1.273.json`; each case swaps its `tool_name`,
//! `tool_input` and `cwd`.
#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use claude_sdk_rs::confine::{CONFINE_DENY_REASON, CONFINE_DISALLOWED_TOOLS};
use claude_sdk_rs::Config;
use serde_json::{json, Value};

const REFERENCE_STDIN: &str = include_str!("fixtures/hook-pretooluse-stdin-2.1.273.json");

/// A root holding one file and a subdir, a symlink inside it pointing out, and a sibling dir
/// outside it. Paths are canonicalized so macOS `/var` vs `/private/var` is exercised by the
/// non-canonical `root` the config receives.
struct Layout {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
}

fn layout() -> Layout {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("notes.txt"), "hello").unwrap();
    std::fs::write(outside.join("secret.txt"), "no").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    Layout {
        _tmp: tmp,
        root,
        outside,
    }
}

fn settings(config: &Config) -> Value {
    let args = config.build_args("hi");
    let idx = args
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings");
    serde_json::from_str(&args[idx + 1]).unwrap()
}

fn confine_command(root: &Path) -> String {
    let config = Config {
        confine_to: Some(root.to_path_buf()),
        ..Config::default()
    };
    let entries = settings(&config)["hooks"]["PreToolUse"].clone();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    entries[0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Run `command` with `stdin`; `true` when it printed a block decision.
fn blocks(command: &str, stdin: &str) -> bool {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "hook exited non-zero: {out:?}");
    let reply: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("hook stdout not JSON ({e}): {out:?}"));
    match reply.get("decision") {
        Some(d) => {
            assert_eq!(d, "block");
            assert_eq!(reply["reason"], CONFINE_DENY_REASON);
            true
        }
        None => {
            assert_eq!(reply, json!({}));
            false
        }
    }
}

fn stdin(tool: &str, input: Value, cwd: &Path) -> String {
    let mut v: Value = serde_json::from_str(REFERENCE_STDIN).unwrap();
    v["tool_name"] = json!(tool);
    v["tool_input"] = input;
    v["cwd"] = json!(cwd.to_string_lossy());
    v.to_string()
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn reference_fixture_shape_is_what_the_cases_mutate() {
    let v: Value = serde_json::from_str(REFERENCE_STDIN).unwrap();
    assert_eq!(v["hook_event_name"], "PreToolUse");
    assert!(v["tool_input"]["file_path"].is_string());
    assert!(v["cwd"].is_string());
}

#[test]
fn file_tools_allowed_inside_root_and_blocked_outside() {
    let l = layout();
    let cmd = confine_command(&l.root);
    let inside = p(&l.root.join("notes.txt"));
    let new_file = p(&l.root.join("sub/new.txt"));
    let outside = p(&l.outside.join("secret.txt"));
    let cases = [
        ("Read", json!({"file_path": inside}), false),
        ("Read", json!({"file_path": "/etc/hosts"}), true),
        ("Read", json!({"file_path": outside}), true),
        (
            "Edit",
            json!({"file_path": inside, "old_string": "a", "new_string": "b"}),
            false,
        ),
        (
            "Edit",
            json!({"file_path": outside, "old_string": "a", "new_string": "b"}),
            true,
        ),
        (
            "Write",
            json!({"file_path": new_file, "content": "x"}),
            false,
        ),
        (
            "Write",
            json!({"file_path": p(&l.outside.join("new.txt")), "content": "x"}),
            true,
        ),
        (
            "MultiEdit",
            json!({"file_path": inside, "edits": []}),
            false,
        ),
        (
            "MultiEdit",
            json!({"file_path": outside, "edits": []}),
            true,
        ),
        (
            "NotebookEdit",
            json!({"notebook_path": p(&l.root.join("a.ipynb")), "new_source": ""}),
            false,
        ),
        (
            "NotebookEdit",
            json!({"notebook_path": p(&l.outside.join("a.ipynb")), "new_source": ""}),
            true,
        ),
    ];
    for (tool, input, want_block) in cases {
        let got = blocks(&cmd, &stdin(tool, input.clone(), &l.root));
        assert_eq!(got, want_block, "{tool} {input}");
    }
}

#[test]
fn escapes_are_blocked() {
    let l = layout();
    let cmd = confine_command(&l.root);
    let cases = [
        // relative path climbing out of cwd
        ("Read", json!({"file_path": "../outside/secret.txt"})),
        // absolute path with .. that leaves the root
        (
            "Read",
            json!({"file_path": format!("{}/../outside/secret.txt", p(&l.root))}),
        ),
        // symlink inside the root pointing out
        (
            "Read",
            json!({"file_path": p(&l.root.join("escape/secret.txt"))}),
        ),
        (
            "Grep",
            json!({"pattern": "x", "path": p(&l.root.join("escape"))}),
        ),
        // root-prefix trick: /tmp/x/root-evil is not inside /tmp/x/root
        (
            "Read",
            json!({"file_path": format!("{}-evil/f", p(&l.root))}),
        ),
    ];
    for (tool, input) in cases {
        assert!(
            blocks(&cmd, &stdin(tool, input.clone(), &l.root)),
            "{tool} {input}"
        );
    }
}

#[test]
fn canonical_cli_paths_match_a_non_canonical_root() {
    // The CLI reports `/private/var/...` on macOS while a temp root is `/var/...`.
    let l = layout();
    let cmd = confine_command(&l.root);
    let canonical_root = std::fs::canonicalize(&l.root).unwrap();
    let file = p(&canonical_root.join("notes.txt"));
    assert!(!blocks(
        &cmd,
        &stdin("Read", json!({"file_path": file}), &canonical_root)
    ));
}

#[test]
fn relative_path_inside_root_is_allowed() {
    let l = layout();
    let cmd = confine_command(&l.root);
    assert!(!blocks(
        &cmd,
        &stdin("Read", json!({"file_path": "notes.txt"}), &l.root)
    ));
}

#[test]
fn missing_or_malformed_required_path_is_blocked() {
    let l = layout();
    let cmd = confine_command(&l.root);
    for (tool, input) in [
        ("Read", json!({})),
        ("Read", json!({"file_path": ""})),
        ("Read", json!({"file_path": 7})),
        ("Write", json!({"content": "x"})),
        ("NotebookEdit", json!({"new_source": ""})),
        // the wrong key for the tool
        (
            "NotebookEdit",
            json!({"file_path": p(&l.root.join("a.ipynb"))}),
        ),
    ] {
        assert!(
            blocks(&cmd, &stdin(tool, input.clone(), &l.root)),
            "{tool} {input}"
        );
    }
}

#[test]
fn grep_and_glob_without_path_follow_cwd() {
    let l = layout();
    let cmd = confine_command(&l.root);
    for tool in ["Grep", "Glob"] {
        let input = json!({"pattern": "*.txt"});
        assert!(
            !blocks(&cmd, &stdin(tool, input.clone(), &l.root)),
            "{tool} cwd inside"
        );
        assert!(
            !blocks(&cmd, &stdin(tool, input.clone(), &l.root.join("sub"))),
            "{tool} cwd in subdir"
        );
        assert!(
            blocks(&cmd, &stdin(tool, input, &l.outside)),
            "{tool} cwd outside"
        );
    }
}

#[test]
fn grep_and_glob_path_is_checked() {
    let l = layout();
    let cmd = confine_command(&l.root);
    for tool in ["Grep", "Glob"] {
        let ok = json!({"pattern": "*.txt", "path": p(&l.root.join("sub"))});
        let bad = json!({"pattern": "*.txt", "path": "/etc"});
        assert!(
            !blocks(&cmd, &stdin(tool, ok, &l.root)),
            "{tool} path inside"
        );
        assert!(
            blocks(&cmd, &stdin(tool, bad, &l.root)),
            "{tool} path outside"
        );
    }
}

#[test]
fn glob_pattern_cannot_escape_the_search_root() {
    let l = layout();
    let cmd = confine_command(&l.root);
    for pattern in ["/etc/*", "../outside/*", "sub/**/../../../outside/*"] {
        let input = json!({"pattern": pattern});
        assert!(blocks(&cmd, &stdin("Glob", input, &l.root)), "{pattern}");
    }
    let inside_abs = format!("{}/**/*.txt", p(&l.root));
    for pattern in ["**/*.rs", "notes.txt", inside_abs.as_str()] {
        let input = json!({"pattern": pattern});
        assert!(!blocks(&cmd, &stdin("Glob", input, &l.root)), "{pattern}");
    }
}

#[test]
fn malformed_stdin_fails_closed() {
    let l = layout();
    let cmd = confine_command(&l.root);
    assert!(blocks(&cmd, "not json"));
    assert!(blocks(&cmd, "{}"));
    let mut no_cwd: Value =
        serde_json::from_str(&stdin("Read", json!({"file_path": "x"}), &l.root)).unwrap();
    no_cwd.as_object_mut().unwrap().remove("cwd");
    assert!(blocks(&cmd, &no_cwd.to_string()));
}

#[test]
fn root_with_a_single_quote_survives_shell_quoting() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("it's root");
    std::fs::create_dir_all(&root).unwrap();
    let cmd = confine_command(&root);
    let inside = p(&root.join("f.txt"));
    assert!(!blocks(
        &cmd,
        &stdin("Read", json!({"file_path": inside}), &root)
    ));
    assert!(blocks(
        &cmd,
        &stdin("Read", json!({"file_path": "/etc/hosts"}), &root)
    ));
}

#[test]
fn confine_disallows_network_and_subagent_tools_once() {
    let config = Config {
        confine_to: Some(PathBuf::from("/tmp")),
        disallowed_tools: vec!["WebFetch".into(), "Bash".into()],
        ..Config::default()
    };
    let args = config.build_args("hi");
    let disallowed: Vec<&str> = args
        .windows(2)
        .filter(|w| w[0] == "--disallowedTools")
        .map(|w| w[1].as_str())
        .collect();
    assert_eq!(disallowed[..2], ["WebFetch", "Bash"]);
    for tool in CONFINE_DISALLOWED_TOOLS {
        assert_eq!(
            disallowed.iter().filter(|t| *t == tool).count(),
            1,
            "{tool}"
        );
    }
    assert!(
        !args.contains(&"--allowedTools".to_string()),
        "Bash untouched"
    );
}

#[test]
fn confine_composes_with_the_deny_list() {
    let config = Config {
        confine_to: Some(PathBuf::from("/tmp")),
        denied_path_patterns: vec!["*/.env".into()],
        ..Config::default()
    };
    let entries = settings(&config)["hooks"]["PreToolUse"].clone();
    let matchers: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["matcher"].as_str().unwrap())
        .collect();
    assert_eq!(
        matchers,
        [
            "Read|Grep|Glob",
            "Read|Edit|Write|MultiEdit|NotebookEdit|Grep|Glob"
        ]
    );
}

#[test]
fn relative_root_resolves_against_cwd() {
    let l = layout();
    let config = Config {
        confine_to: Some(PathBuf::from("root")),
        cwd: Some(l.root.parent().unwrap().to_path_buf()),
        ..Config::default()
    };
    let cmd = settings(&config)["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    let inside = p(&l.root.join("notes.txt"));
    assert!(!blocks(
        &cmd,
        &stdin("Read", json!({"file_path": inside}), &l.root)
    ));
    assert!(blocks(
        &cmd,
        &stdin(
            "Read",
            json!({"file_path": p(&l.outside.join("secret.txt"))}),
            &l.root
        )
    ));
}
