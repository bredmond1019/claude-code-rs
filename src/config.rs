//! Subprocess configuration for the `claude` CLI and its argv builder.

/// Configuration for a single `claude` CLI invocation.
///
/// Covers the ported flags from `claude-sdk-rs`'s config plus the env/cwd
/// override fields and the opt-in credential isolation switch consumed by
/// `execute()` (`CC.1.B`).
// The flags map one-to-one onto independent `claude` CLI options, so grouping
// them into a sub-struct would not improve clarity.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Optional system prompt (`--system-prompt`).
    pub system_prompt: Option<String>,

    /// Optional text appended to the default system prompt (`--append-system-prompt`).
    pub append_system_prompt: Option<String>,

    /// Optional model override (`--model`).
    pub model: Option<String>,

    /// Tools explicitly allowed (`--allowedTools`, one flag per tool).
    pub allowed_tools: Vec<String>,

    /// Tools explicitly disallowed (`--disallowedTools`, one flag per tool).
    pub disallowed_tools: Vec<String>,

    /// When `true`, resumes the most recent session (`--continue`).
    pub continue_session: bool,

    /// When set, resumes the given session id (`--resume <id>`).
    pub resume: Option<String>,

    /// Working directory override for the spawned `claude` process. When set,
    /// `execute()` applies it via `Command::current_dir`.
    pub cwd: Option<std::path::PathBuf>,

    /// Extra environment variables applied to the spawned `claude` process via
    /// `Command::envs`, on top of the inherited environment.
    pub env: Vec<(String, String)>,

    /// Opt-in credential isolation switch. When `true`, `execute()` runs the
    /// subprocess under a temp `CLAUDE_CONFIG_DIR` with a redacted copy of the
    /// credentials (see the `isolation` module), so a concurrent subprocess
    /// session cannot log out an interactive session. Defaults to `false`
    /// (inherited env, no isolation) to keep the default execution path
    /// unchanged.
    pub isolated: bool,

    /// When `true`, appends `--dangerously-skip-permissions` so the spawned
    /// session never blocks on an interactive tool-use approval prompt —
    /// required for any headless (`-p`) run that needs to actually use a
    /// file-editing/bash tool, since there is no TTY to approve one on.
    /// Defaults to `false` (today's text-only-response behavior unchanged).
    /// Callers that opt in are responsible for scoping the blast radius
    /// themselves, e.g. via `cwd` plus `disallowed_tools` — this flag alone
    /// grants no tool a wider reach than the CLI's own tool definitions
    /// allow.
    pub dangerously_skip_permissions: bool,
}

impl Config {
    /// Build the exact argv (excluding the binary itself) for a single `claude` call.
    ///
    /// Order: `-p <prompt>`, `--system-prompt`, `--append-system-prompt`, `--model`,
    /// `--allowedTools` (repeated), `--disallowedTools` (repeated), `--continue`,
    /// `--resume <id>`, `--dangerously-skip-permissions`, then always
    /// `--output-format json`.
    #[must_use]
    pub fn build_args(&self, prompt: &str) -> Vec<String> {
        let mut args = Vec::new();

        args.push("-p".to_string());
        args.push(prompt.to_string());

        if let Some(system_prompt) = &self.system_prompt {
            args.push("--system-prompt".to_string());
            args.push(system_prompt.clone());
        }

        if let Some(append_system_prompt) = &self.append_system_prompt {
            args.push("--append-system-prompt".to_string());
            args.push(append_system_prompt.clone());
        }

        if let Some(model) = &self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        for tool in &self.allowed_tools {
            args.push("--allowedTools".to_string());
            args.push(tool.clone());
        }

        for tool in &self.disallowed_tools {
            args.push("--disallowedTools".to_string());
            args.push(tool.clone());
        }

        if self.continue_session {
            args.push("--continue".to_string());
        }

        if let Some(resume) = &self.resume {
            args.push("--resume".to_string());
            args.push(resume.clone());
        }

        if self.dangerously_skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }

        args.push("--output-format".to_string());
        args.push("json".to_string());

        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_config_appends_output_format() {
        let config = Config::default();
        let args = config.build_args("hello");
        assert_eq!(
            args,
            vec!["-p", "hello", "--output-format", "json"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn full_config_orders_flags() {
        let config = Config {
            system_prompt: Some("be helpful".to_string()),
            append_system_prompt: Some("also be terse".to_string()),
            model: Some("claude-opus-4".to_string()),
            allowed_tools: vec!["bash".to_string()],
            disallowed_tools: vec!["web".to_string()],
            continue_session: true,
            resume: Some("session-123".to_string()),
            dangerously_skip_permissions: true,
            ..Config::default()
        };

        let args = config.build_args("hi");

        assert_eq!(
            args,
            vec![
                "-p",
                "hi",
                "--system-prompt",
                "be helpful",
                "--append-system-prompt",
                "also be terse",
                "--model",
                "claude-opus-4",
                "--allowedTools",
                "bash",
                "--disallowedTools",
                "web",
                "--continue",
                "--resume",
                "session-123",
                "--dangerously-skip-permissions",
                "--output-format",
                "json",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn dangerously_skip_permissions_defaults_to_false_and_omits_flag() {
        let config = Config::default();
        assert!(!config.dangerously_skip_permissions);
        assert!(!config
            .build_args("hi")
            .contains(&"--dangerously-skip-permissions".to_string()));
    }
}
