//! Exact-argv lock test for `Config::build_args`.
//!
//! Asserts the precise `Vec<String>` produced by the argv builder — flag order,
//! flag names, and the trailing `--output-format json` — so any drift in the
//! builder's shape is caught here rather than downstream in `execute()`.

use claude_code_rs::Config;

#[test]
fn minimal_config_builds_prompt_and_output_format_only() {
    let config = Config::default();

    let args = config.build_args("hello world");

    assert_eq!(
        args,
        vec!["-p", "hello world", "--output-format", "json"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn full_config_builds_exact_argv_in_order() {
    let config = Config {
        system_prompt: Some("be helpful".to_string()),
        append_system_prompt: Some("also be terse".to_string()),
        model: Some("claude-opus-4".to_string()),
        allowed_tools: vec!["bash".to_string(), "web".to_string()],
        disallowed_tools: vec!["rm".to_string()],
        continue_session: true,
        resume: Some("session-abc-123".to_string()),
        ..Config::default()
    };

    let args = config.build_args("plan the migration");

    assert_eq!(
        args,
        vec![
            "-p",
            "plan the migration",
            "--system-prompt",
            "be helpful",
            "--append-system-prompt",
            "also be terse",
            "--model",
            "claude-opus-4",
            "--allowedTools",
            "bash",
            "--allowedTools",
            "web",
            "--disallowedTools",
            "rm",
            "--continue",
            "--resume",
            "session-abc-123",
            "--output-format",
            "json",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>()
    );
}

#[test]
fn resume_without_continue_is_included_without_continue_flag() {
    let config = Config {
        resume: Some("session-xyz".to_string()),
        ..Config::default()
    };

    let args = config.build_args("resume this");

    assert_eq!(
        args,
        vec![
            "-p",
            "resume this",
            "--resume",
            "session-xyz",
            "--output-format",
            "json",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>()
    );
}
