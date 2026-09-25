//! Subprocess configuration for the `claude` CLI and its argv builder.

use std::time::Duration;

/// The CLI's `--permission-mode <mode>` values, as a closed set so an unknown
/// value is a compile error rather than a string that silently fails to
/// match the CLI's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionMode {
    /// `acceptEdits` — auto-accept file edits.
    AcceptEdits,
    /// `auto` — the CLI's standard interactive prompting.
    Auto,
    /// `bypassPermissions` — skip all permission checks.
    BypassPermissions,
    /// `manual` — prompt for every tool use.
    Manual,
    /// `dontAsk` — deny any tool use not covered by an allow-list, never
    /// prompting. The mode `Config::permission_mode` exists to support:
    /// headless callers have no TTY to answer a prompt on, so `dontAsk` plus
    /// an explicit `allowed_tools` list is the only way to grant a scoped
    /// set of tools without hanging.
    DontAsk,
    /// `plan` — plan-only mode; no tool execution.
    Plan,
}

impl PermissionMode {
    /// Every variant, in declaration order — used by round-trip tests and by
    /// any caller that needs to enumerate the full set.
    pub const ALL: &'static [PermissionMode] = &[
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::BypassPermissions,
        PermissionMode::Manual,
        PermissionMode::DontAsk,
        PermissionMode::Plan,
    ];

    /// The exact CLI spelling for this mode's `--permission-mode` value —
    /// the single source of truth other code and `Display` delegate to.
    #[must_use]
    pub fn as_cli_str(&self) -> &'static str {
        match self {
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
            PermissionMode::BypassPermissions => "bypassPermissions",
            PermissionMode::Manual => "manual",
            PermissionMode::DontAsk => "dontAsk",
            PermissionMode::Plan => "plan",
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_cli_str())
    }
}

/// Error returned by [`PermissionMode::from_str`] for a string that does not
/// match any of the CLI's six `--permission-mode` values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePermissionModeError(pub String);

impl std::fmt::Display for ParsePermissionModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown permission mode: {}", self.0)
    }
}

impl std::error::Error for ParsePermissionModeError {}

impl std::str::FromStr for PermissionMode {
    type Err = ParsePermissionModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "acceptEdits" => Ok(PermissionMode::AcceptEdits),
            "auto" => Ok(PermissionMode::Auto),
            "bypassPermissions" => Ok(PermissionMode::BypassPermissions),
            "manual" => Ok(PermissionMode::Manual),
            "dontAsk" => Ok(PermissionMode::DontAsk),
            "plan" => Ok(PermissionMode::Plan),
            other => Err(ParsePermissionModeError(other.to_string())),
        }
    }
}

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

    /// Opt-in: when a `config.isolated` call fails with the exact shape of an
    /// expired/invalid isolated credential snapshot (see
    /// `heal::is_isolated_auth_expired`), `execute()` attempts a best-effort
    /// heal of the real shared credentials and retries exactly once through a
    /// fresh `IsolatedConfigDir`.
    ///
    /// This is **not** a CLI flag — it never appears in [`Config::build_args`]
    /// output. Defaults to `false`, preserving today's hard-fail behavior for
    /// every caller that does not opt in.
    pub heal_isolated_auth_on_expiry: bool,

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

    /// Optional permission mode override (`--permission-mode <mode>`).
    ///
    /// `None` (the default) omits the flag entirely, so every existing
    /// caller's argv is byte-identical. `Some(mode)` emits
    /// `--permission-mode` followed by [`PermissionMode::as_cli_str`]'s CLI
    /// spelling. Alternative to [`Config::dangerously_skip_permissions`] —
    /// [`Config::validate`] rejects a `Config` that sets both, since
    /// combining them is ambiguous about which permission behavior wins.
    pub permission_mode: Option<PermissionMode>,

    /// Optional JSON Schema to enforce on Claude's reply (`--json-schema
    /// <json>`). When `Some`, `build_args` serializes it to compact JSON and
    /// emits the flag immediately before the trailing `--output-format json`
    /// pair; when `None`, the flag is omitted entirely (today's schemaless
    /// behavior unchanged).
    pub json_schema: Option<serde_json::Value>,

    /// Optional ceiling on the number of agentic turns a single `claude`
    /// invocation may take (`--max-turns <n>`). When `Some(n)`, `build_args`
    /// emits `--max-turns` followed by `n`; when `None` (the default), the
    /// flag is omitted entirely and the CLI's own default turn behavior
    /// applies (today's unbounded-turn behavior unchanged).
    pub max_turns: Option<u32>,

    /// Optional list of setting sources the CLI loads (`--setting-sources=<list>`).
    ///
    /// Controls which settings layers (`user`, `project`, `local`) a call loads,
    /// and with them the CLAUDE.md/AGENTS.md chain and project hooks.
    /// `None` (the default) omits the flag, so every existing caller's behavior
    /// is unchanged. `Some(vec![])` emits `--setting-sources=` (load none): a
    /// measured first-turn context of ~21K tokens against ~48K by default in
    /// this fleet. `Some(sources)` emits them comma-joined.
    ///
    /// Always emitted as a single `--setting-sources=<value>` token. A separate
    /// empty argument makes the CLI read the NEXT flag as the value.
    pub setting_sources: Option<Vec<String>>,

    /// Optional override for `execute()`'s whole-call timeout.
    ///
    /// This is **not** a CLI flag — it never appears in [`Config::build_args`]
    /// output. It only sets the duration of the Rust-side
    /// [`tokio::time::timeout`](https://docs.rs/tokio/latest/tokio/time/fn.timeout.html)
    /// that wraps the spawn-and-wait of the `claude` subprocess.
    ///
    /// `None` (the `#[derive(Default)]` value) preserves `execute()`'s built-in
    /// `DEFAULT_TIMEOUT` of 300 seconds, so every existing caller's behavior is
    /// unchanged. `Some(duration)` widens or narrows that timeout for this call
    /// — useful for long-running agentic work (multi-file writes, cold
    /// worktrees) that legitimately exceeds five minutes.
    pub timeout: Option<Duration>,

    /// Optional override for the whole-call timeout applied to the best-effort
    /// shared-credential heal triggered by [`Config::heal_isolated_auth_on_expiry`].
    ///
    /// This is **not** a CLI flag — it never appears in [`Config::build_args`]
    /// output. `None` (the `#[derive(Default)]` value) uses the built-in
    /// `DEFAULT_HEAL_TIMEOUT` (15 seconds); `Some(duration)` overrides it.
    /// Mirrors [`Config::timeout`]'s `None`-means-built-in-default shape.
    pub heal_timeout: Option<Duration>,
}

impl Config {
    /// Build the exact argv (excluding the binary itself) for a single `claude` call.
    ///
    /// Order: `-p <prompt>`, `--system-prompt`, `--append-system-prompt`, `--model`,
    /// `--allowedTools` (repeated), `--disallowedTools` (repeated), `--continue`,
    /// `--resume <id>`, `--dangerously-skip-permissions`, `--permission-mode <mode>`,
    /// `--json-schema <json>`, `--max-turns <n>`, `--setting-sources=<list>`, then
    /// always `--output-format json`.
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

        if let Some(mode) = self.permission_mode {
            args.push("--permission-mode".to_string());
            args.push(mode.as_cli_str().to_string());
        }

        if let Some(json_schema) = &self.json_schema {
            args.push("--json-schema".to_string());
            args.push(json_schema.to_string());
        }

        if let Some(max_turns) = self.max_turns {
            args.push("--max-turns".to_string());
            args.push(max_turns.to_string());
        }

        if let Some(setting_sources) = &self.setting_sources {
            args.push(format!("--setting-sources={}", setting_sources.join(",")));
        }

        args.push("--output-format".to_string());
        args.push("json".to_string());

        args
    }

    /// Checks for conflicting settings that `build_args` cannot express
    /// safely. `execute()` calls this first, before resolving the binary, so
    /// a conflicting `Config` never spawns a subprocess.
    ///
    /// # Errors
    /// [`crate::error::Error::ConflictingPermissions`] if both
    /// `dangerously_skip_permissions` and `permission_mode` are set —
    /// they are alternative ways of controlling permissions and combining
    /// them is ambiguous about which one wins.
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.dangerously_skip_permissions && self.permission_mode.is_some() {
            return Err(crate::error::Error::ConflictingPermissions);
        }
        Ok(())
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
            json_schema: Some(serde_json::json!({"type": "object"})),
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
                "--json-schema",
                "{\"type\":\"object\"}",
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

    #[test]
    fn json_schema_defaults_to_none_and_omits_flag() {
        let config = Config::default();
        assert!(config.json_schema.is_none());
        assert!(!config
            .build_args("hi")
            .contains(&"--json-schema".to_string()));
    }

    #[test]
    fn default_max_turns_is_none() {
        assert!(Config::default().max_turns.is_none());
    }

    #[test]
    fn build_args_omits_max_turns_by_default() {
        let config = Config::default();
        assert!(!config.build_args("hi").contains(&"--max-turns".to_string()));
    }

    #[test]
    fn build_args_emits_max_turns_when_set() {
        let config = Config {
            max_turns: Some(3),
            ..Config::default()
        };
        let args = config.build_args("hi");

        let count = args.iter().filter(|a| *a == "--max-turns").count();
        assert_eq!(count, 1);

        let idx = args
            .iter()
            .position(|a| a == "--max-turns")
            .expect("--max-turns must be present");
        assert_eq!(args[idx + 1], "3");
    }

    #[test]
    fn build_args_omits_setting_sources_by_default() {
        let config = Config::default();
        assert!(config.setting_sources.is_none());
        assert!(!config
            .build_args("hi")
            .iter()
            .any(|a| a.starts_with("--setting-sources")));
    }

    #[test]
    fn build_args_emits_empty_setting_sources_as_one_token() {
        let config = Config {
            setting_sources: Some(Vec::new()),
            ..Config::default()
        };
        let args = config.build_args("hi");

        let matches: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("--setting-sources"))
            .collect();
        assert_eq!(matches, vec!["--setting-sources="]);
        let idx = args
            .iter()
            .position(|a| a == "--setting-sources=")
            .expect("--setting-sources= must be present");
        assert_eq!(args[idx + 1], "--output-format");
    }

    #[test]
    fn heal_isolated_auth_on_expiry_defaults_to_false_and_heal_timeout_to_none() {
        let config = Config::default();
        assert!(!config.heal_isolated_auth_on_expiry);
        assert!(config.heal_timeout.is_none());
    }

    #[test]
    fn heal_fields_never_appear_in_build_args() {
        let config = Config {
            heal_isolated_auth_on_expiry: true,
            heal_timeout: Some(Duration::from_secs(5)),
            ..Config::default()
        };
        let args = config.build_args("hi");
        let default_args = Config::default().build_args("hi");
        assert_eq!(
            args, default_args,
            "heal_isolated_auth_on_expiry and heal_timeout must never affect build_args output"
        );
    }

    #[test]
    fn permission_mode_defaults_to_none_and_omits_flag() {
        let config = Config::default();
        assert!(config.permission_mode.is_none());
        assert!(!config
            .build_args("hi")
            .contains(&"--permission-mode".to_string()));
    }

    #[test]
    fn build_args_emits_permission_mode_and_allowed_tools_when_set() {
        let config = Config {
            permission_mode: Some(PermissionMode::DontAsk),
            allowed_tools: vec!["Edit".to_string(), "Bash(git rm --cached:*)".to_string()],
            ..Config::default()
        };

        let args = config.build_args("hi");

        assert_eq!(
            args,
            vec![
                "-p",
                "hi",
                "--allowedTools",
                "Edit",
                "--allowedTools",
                "Bash(git rm --cached:*)",
                "--permission-mode",
                "dontAsk",
                "--output-format",
                "json",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn permission_mode_round_trips_through_cli_str_for_every_variant() {
        use std::str::FromStr;
        for mode in PermissionMode::ALL {
            assert_eq!(PermissionMode::from_str(mode.as_cli_str()).unwrap(), *mode);
        }
    }

    #[test]
    fn permission_mode_from_str_rejects_unknown_value() {
        use std::str::FromStr;
        assert_eq!(
            PermissionMode::from_str("not-a-mode"),
            Err(ParsePermissionModeError("not-a-mode".to_string()))
        );
    }

    #[test]
    fn validate_rejects_both_dangerously_skip_permissions_and_permission_mode() {
        let config = Config {
            dangerously_skip_permissions: true,
            permission_mode: Some(PermissionMode::Auto),
            ..Config::default()
        };
        assert!(matches!(
            config.validate(),
            Err(crate::error::Error::ConflictingPermissions)
        ));
    }

    #[test]
    fn validate_accepts_dangerously_skip_permissions_alone() {
        let config = Config {
            dangerously_skip_permissions: true,
            ..Config::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_permission_mode_alone() {
        let config = Config {
            permission_mode: Some(PermissionMode::DontAsk),
            ..Config::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn build_args_joins_setting_sources_with_commas() {
        let config = Config {
            setting_sources: Some(vec!["user".to_string(), "project".to_string()]),
            ..Config::default()
        };
        assert!(config
            .build_args("hi")
            .contains(&"--setting-sources=user,project".to_string()));
    }
}
