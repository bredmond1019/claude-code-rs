//! Caller-supplied command hooks (`Config::pre_tool_use_hooks`): a matcher plus a shell command the
//! CLI runs before a tool call, rendered as one more `PreToolUse` entry in the single `--settings`
//! object next to the deny-list and allow-root entries.
//!
//! Both strings are opaque caller data. This crate carries no hook path or command of its own.

use serde_json::{json, Value};

use crate::{Error, Result};

/// One `PreToolUse` command hook: which tools it matches and the command it runs.
///
/// Fields are private and [`CommandHook::new`] is the only constructor, so an empty matcher or
/// command cannot exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandHook {
    matcher: String,
    command: String,
}

impl CommandHook {
    /// Build a hook, refusing an empty (or whitespace-only) matcher or command with
    /// [`Error::InvalidCommandHook`] naming the offending field.
    ///
    /// # Errors
    /// Returns [`Error::InvalidCommandHook`] when `matcher` or `command` is blank.
    pub fn new(matcher: impl Into<String>, command: impl Into<String>) -> Result<Self> {
        let matcher = matcher.into();
        let command = command.into();
        if matcher.trim().is_empty() {
            return Err(Error::InvalidCommandHook { field: "matcher" });
        }
        if command.trim().is_empty() {
            return Err(Error::InvalidCommandHook { field: "command" });
        }
        Ok(Self { matcher, command })
    }

    /// The tool-name matcher, verbatim.
    #[must_use]
    pub fn matcher(&self) -> &str {
        &self.matcher
    }

    /// The shell command, verbatim.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// The `PreToolUse` entry, shaped like the deny-list and confine entries.
    // Consumed by `settings::cli_settings` in the next task of this block.
    #[allow(dead_code)]
    pub(crate) fn hook_entry(&self) -> Value {
        json!({
            "matcher": self.matcher,
            "hooks": [{ "type": "command", "command": self.command }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_round_trips_accessors() {
        let h = CommandHook::new("Bash", "/x/hook.sh --flag").unwrap();
        assert_eq!(h.matcher(), "Bash");
        assert_eq!(h.command(), "/x/hook.sh --flag");
    }

    #[test]
    fn empty_matcher_is_refused_naming_the_field() {
        for m in ["", "   "] {
            let err = CommandHook::new(m, "x").unwrap_err();
            assert!(matches!(
                err,
                Error::InvalidCommandHook { field: "matcher" }
            ));
        }
    }

    #[test]
    fn empty_command_is_refused_naming_the_field() {
        for c in ["", "\t "] {
            let err = CommandHook::new("Bash", c).unwrap_err();
            assert!(matches!(
                err,
                Error::InvalidCommandHook { field: "command" }
            ));
        }
    }

    #[test]
    fn hook_entry_has_the_existing_shape_verbatim() {
        let h = CommandHook::new("Bash|Edit", "/x/hook.sh").unwrap();
        assert_eq!(
            h.hook_entry(),
            json!({
                "matcher": "Bash|Edit",
                "hooks": [{ "type": "command", "command": "/x/hook.sh" }],
            })
        );
    }
}
