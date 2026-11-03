//! Anthropic-compatible endpoint override (`Config::endpoint`).
//!
//! Points the spawned `claude` at a non-Anthropic server that speaks the Messages API — Ollama's
//! `/v1/messages`, or a logging proxy in front of it — with no route back to Anthropic: the real
//! API key is removed, a placeholder bearer token is sent instead, and every model-selection
//! variable the CLI reads is pinned to the one tag the server serves, so no background call
//! (title generation, subagents, classifiers) asks for a model the server lacks.
//!
//! The variable names below were read out of the installed CLI binary (2.1.317). They are the only
//! place those names live; `execute()` and the `--settings` builder both read this table.

use serde_json::{Map, Value};

/// The bearer token [`Endpoint::new`] sends. An Ollama daemon that is signed in needs no key, but
/// the CLI refuses to start without some credential, so a fixed placeholder fills the slot.
pub const DEFAULT_PLACEHOLDER_AUTH_TOKEN: &str = "ollama";

/// Set to [`Endpoint::base_url`].
pub const BASE_URL_ENV_VAR: &str = "ANTHROPIC_BASE_URL";

/// Set to [`Endpoint::auth_token`].
pub const AUTH_TOKEN_ENV_VAR: &str = "ANTHROPIC_AUTH_TOKEN";

/// Removed from the child's environment (and blanked in the `--settings` `env` block) so a real
/// Anthropic key can never be sent, to the endpoint or anywhere else.
pub const REMOVED_ENV_VARS: &[&str] = &["ANTHROPIC_API_KEY"];

/// Every variable the CLI (2.1.317) reads to pick a model, each set to [`Endpoint::model`].
pub const MODEL_ENV_VARS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
    "CLAUDE_CODE_AUTO_MODE_MODEL",
    "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
];

/// Fixed-value switches set whenever an endpoint is in use: no telemetry, error reporting,
/// auto-update or feature-flag traffic to Anthropic's own hosts.
pub const FIXED_ENV_VARS: &[(&str, &str)] = &[("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")];

/// An Anthropic-compatible server the spawned `claude` talks to instead of Anthropic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Server root, e.g. `http://127.0.0.1:11434` (Ollama) — the CLI appends `/v1/messages`.
    pub base_url: String,
    /// The one model tag the server serves, e.g. `gpt-oss:120b-cloud`. Passed as `--model` and
    /// as every [`MODEL_ENV_VARS`] entry.
    pub model: String,
    /// Bearer token sent as `ANTHROPIC_AUTH_TOKEN`. Never a real Anthropic credential.
    pub auth_token: String,
}

impl Endpoint {
    /// An endpoint carrying [`DEFAULT_PLACEHOLDER_AUTH_TOKEN`].
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            auth_token: DEFAULT_PLACEHOLDER_AUTH_TOKEN.to_string(),
        }
    }

    /// The variables to set on the child, in table order.
    #[must_use]
    pub fn env_vars(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            (BASE_URL_ENV_VAR.to_string(), self.base_url.clone()),
            (AUTH_TOKEN_ENV_VAR.to_string(), self.auth_token.clone()),
        ];
        vars.extend(
            MODEL_ENV_VARS
                .iter()
                .map(|name| ((*name).to_string(), self.model.clone())),
        );
        vars.extend(
            FIXED_ENV_VARS
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        );
        vars
    }

    /// The `env` block for the inline `--settings` JSON. Settings `env` blocks override the
    /// process environment, and command-line settings outrank user, project and local ones, so
    /// repeating the variables here stops a `settings.json` in `cwd` from re-pointing the call.
    /// Removed variables are blanked, since a settings block cannot unset one.
    pub(crate) fn settings_env(&self) -> Value {
        let mut env: Map<String, Value> = self
            .env_vars()
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        for name in REMOVED_ENV_VARS {
            env.insert((*name).to_string(), Value::String(String::new()));
        }
        Value::Object(env)
    }
}
