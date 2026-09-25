//! `Outcome` type and parser for the `claude` CLI's `--output-format json` response.
//!
//! The shape parsed here is not asserted from memory — it is defined by the captured fixtures in
//! `tests/fixtures/`, which are real CLI responses. See `tests/fixtures/README.md` for provenance,
//! the capture/re-capture procedure, and the table of which fields this crate deliberately ignores.
//!
//! # Leniency policy
//!
//! Fields are required (loud on absence) or defaulted (lenient) according to one rule: **is absence
//! distinguishable from a legitimate value?**
//!
//! - [`Outcome::text`] is defaulted. It was required until the `error_max_turns` envelope proved
//!   that an absent `result` is a legitimate CLI state on that path, not silent data loss — that
//!   envelope carries no `result` key at all, and a required field there would drop the whole
//!   billed envelope (cost, usage, session_id) into `Error::Parse`. The 2026-07-16 drift this field
//!   used to guard against is unaffected: it was the CLI silently *emptying* `result` on a
//!   `subtype: "success"` reply, which the `unknown_fields_do_not_fail_parse`-style tests and the
//!   live smoke test still catch — this change only stops treating an *absent* `result` on an
//!   error envelope the same as that silent-emptying bug.
//! - [`Outcome::api_error_status`] is defaulted. Its absence costs an HTTP status code on an error
//!   that is still fully described by `is_error` and `text` — less detail, not lost data.
//! - [`Outcome::structured_output`] is defaulted. Its absence means "no schema was requested" — a
//!   schemaless call never has this key at all, so a missing value is a legitimate, expected state,
//!   not lost data.
//!
//! This rule exists because the opposite instinct caused a real bug: a `#[serde(default)]` on a
//! response-text field let the vendor remove it while every test kept passing.
//!
//! Unknown fields are ignored rather than rejected — the CLI adds fields routinely, and
//! `deny_unknown_fields` would turn every vendor release into an outage. Additive drift is caught by
//! the `#[ignore]`d canary in `tests/parse_schema.rs`, which diffs a live response against the fixture.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::Result;

/// Aggregate token usage reported by the `claude` CLI for a single call.
///
/// The CLI's `usage` object carries further fields (`server_tool_use`, `service_tier`,
/// `cache_creation`, `inference_geo`, `iterations`, `speed`) that this crate ignores.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the input (prompt + context), excluding cache reads.
    #[serde(default)]
    pub input_tokens: u64,

    /// Tokens produced in the output.
    #[serde(default)]
    pub output_tokens: u64,

    /// Tokens used to write to the prompt cache.
    #[serde(default)]
    pub cache_creation_input_tokens: u64,

    /// Tokens read from the prompt cache.
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// Per-model usage, as reported in the CLI's `modelUsage` map.
///
/// Note the CLI emits these keys in camelCase, unlike the snake_case top-level `usage` object.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    /// Tokens consumed by the input for this model.
    #[serde(default)]
    pub input_tokens: u64,

    /// Tokens produced by this model.
    #[serde(default)]
    pub output_tokens: u64,

    /// Tokens read from the prompt cache for this model.
    #[serde(default)]
    pub cache_read_input_tokens: u64,

    /// Tokens written to the prompt cache for this model.
    #[serde(default)]
    pub cache_creation_input_tokens: u64,

    /// Cost attributed to this model, in USD.
    #[serde(rename = "costUSD", default)]
    pub cost_usd: f64,
}

/// Parsed result of a single `claude` CLI invocation.
///
/// Covers both the success and error envelopes — they share 17 top-level keys, and `result` is
/// present on both (carrying the reply, or the error message). Distinguish them with [`is_error`],
/// never with the CLI's `subtype` field, which reports `"success"` on both.
///
/// [`is_error`]: Outcome::is_error
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Outcome {
    /// Total cost of the call in USD (CLI: `total_cost_usd`).
    #[serde(rename = "total_cost_usd")]
    pub cost_usd: f64,

    /// Aggregate token usage for the call (CLI: top-level `usage`).
    pub usage: Usage,

    /// Per-model usage, keyed by model name (CLI: `modelUsage`).
    ///
    /// **This map is the only place the model name appears** — there is no top-level `model` field.
    /// Empty on the error envelope. Use [`Outcome::primary_model`] rather than reading it directly.
    ///
    /// A `BTreeMap`, not a `HashMap`: [`Outcome::primary_model`] breaks ties on key order, and
    /// `HashMap`'s per-process iteration randomization would make that tie-break silently flaky.
    #[serde(rename = "modelUsage", default)]
    pub model_usage: BTreeMap<String, ModelUsage>,

    /// The response text (CLI: `result`).
    ///
    /// On the error envelope this carries the human-readable error message instead of a reply.
    ///
    /// Defaulted to `""`, not required, per the leniency policy above: the `error_max_turns`
    /// envelope carries no `result` key at all, and a required field there would fail the whole
    /// parse and lose the billed envelope (cost, usage, session_id) with it.
    #[serde(rename = "result", default)]
    pub text: String,

    /// The envelope's reported subtype (CLI: `subtype`), e.g. `"success"` or `"error_max_turns"`.
    ///
    /// A closed set of known values plus a catch-all [`ResultSubtype::Unknown`] for forward
    /// compatibility — the wire is a vendor contract this crate does not own, so an unrecognised
    /// subtype parses rather than failing. `None` when the envelope carries no `subtype` at all.
    ///
    /// Never use this to distinguish success from failure — [`is_error`] is the only trustworthy
    /// signal for that, per the struct-level doc above.
    ///
    /// [`is_error`]: Outcome::is_error
    #[serde(default)]
    pub subtype: Option<ResultSubtype>,

    /// The number of turns the call took (CLI: `num_turns`).
    ///
    /// Present on both envelopes; most useful alongside [`ResultSubtype::ErrorMaxTurns`], where it
    /// reports how many turns were actually used before the ceiling stopped the call.
    #[serde(default)]
    pub num_turns: Option<u32>,

    /// Structured error messages the CLI attached to this call (CLI: `errors`).
    ///
    /// Empty on a normal success envelope; populated on an error envelope such as
    /// `error_max_turns`, where it carries the human-readable turn-limit message.
    #[serde(default)]
    pub errors: Vec<String>,

    /// Whether the CLI reported a failure.
    ///
    /// The only trustworthy error signal on the envelope.
    pub is_error: bool,

    /// HTTP status of the underlying API error, when [`is_error`] is set.
    ///
    /// `None` on success.
    ///
    /// [`is_error`]: Outcome::is_error
    #[serde(default)]
    pub api_error_status: Option<u16>,

    /// The CLI session this call ran under (CLI: `session_id`).
    ///
    /// Present on **both** the success and the error envelope, so a failed call is still
    /// attributable — a bailed attempt consumed tokens like any other.
    ///
    /// This is the exact join key to the session transcript, which the CLI writes to
    /// `~/.claude/projects/<project>/<session_id>.jsonl` — the id is literally the filename stem,
    /// and the file's own `sessionId` field repeats it. Consumers that attribute real token usage
    /// (rather than an estimate) read that transcript; without this field the join can only be
    /// inferred from a timestamp window.
    ///
    /// Defaulted, not required, per the leniency policy above: an absent `session_id` costs a join
    /// key on a call that is still fully described by `usage`/`text`/`is_error` — less detail, not
    /// lost data — and a required field would turn a CLI that stops emitting it into an outage.
    /// `None` means the envelope carried no id at all; the CLI never emits an empty one.
    #[serde(default)]
    pub session_id: Option<String>,

    /// The parsed structured output, when a `Config.json_schema` was supplied (CLI:
    /// `structured_output`).
    ///
    /// Present only when the call was made with `--json-schema`; absent (not `null`) otherwise. On
    /// the success envelope with a schema, [`Outcome::text`] still carries the same JSON as a string
    /// (via `result`) — this field is the pre-parsed object form.
    #[serde(default)]
    pub structured_output: Option<serde_json::Value>,
}

/// The `claude` CLI's reported envelope subtype (CLI: `subtype`).
///
/// A closed set of the CLI's known values, plus a catch-all [`ResultSubtype::Unknown`] for forward
/// compatibility — the wire is a vendor contract this crate does not own, so an unrecognised
/// subtype parses into `Unknown` rather than failing the whole envelope.
///
/// Never use this to distinguish success from failure — [`Outcome::is_error`] is the only
/// trustworthy signal for that; the envelope can report `subtype: "success"` even on a call that
/// failed for a reason `is_error`/`subtype` disagree on (see the struct-level doc on [`Outcome`]).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum ResultSubtype {
    /// The call completed normally.
    Success,
    /// The call stopped because it hit its turn ceiling (CLI: `error_max_turns`).
    ErrorMaxTurns,
    /// The call failed during execution for a reason other than the turn ceiling.
    ErrorDuringExecution,
    /// A subtype value this crate does not yet recognise, carrying the raw wire string.
    Unknown(String),
}

impl ResultSubtype {
    /// The wire spelling for this subtype — the inverse of `From<String> for ResultSubtype`.
    ///
    /// Built from the same mapping as that `From` impl so the two can never drift apart into two
    /// separate tables of the wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Success => "success",
            Self::ErrorMaxTurns => "error_max_turns",
            Self::ErrorDuringExecution => "error_during_execution",
            Self::Unknown(raw) => raw,
        }
    }
}

impl From<String> for ResultSubtype {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "success" => Self::Success,
            "error_max_turns" => Self::ErrorMaxTurns,
            "error_during_execution" => Self::ErrorDuringExecution,
            _ => Self::Unknown(raw),
        }
    }
}

impl Outcome {
    /// The model that most plausibly produced this response.
    ///
    /// # This is a heuristic, not CLI ground truth
    ///
    /// The CLI reports a *map* of models, not a primary one — a single call can bill several
    /// (delegated sub-agents, background title generation). Picking one is **this crate's policy**,
    /// not something the vendor tells us.
    ///
    /// Ranks by cost, then output tokens, then key order (ascending) as a deterministic final
    /// tiebreak. Cost leads because a cheap background model can out-produce the model that did the
    /// real work by token count, but rarely out-bills it.
    ///
    /// Returns `None` when `modelUsage` is empty — always true on the error envelope. Callers that
    /// need a non-optional label must supply their own fallback.
    #[must_use]
    pub fn primary_model(&self) -> Option<&str> {
        self.model_usage
            .iter()
            .max_by(|(key_a, a), (key_b, b)| {
                a.cost_usd
                    .partial_cmp(&b.cost_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.output_tokens.cmp(&b.output_tokens))
                    // Reversed, so that on a total tie `max_by` yields the
                    // lexicographically *first* key.
                    .then(key_b.cmp(key_a))
            })
            .map(|(key, _)| key.as_str())
    }
}

/// Parse a `claude` CLI JSON response into an [`Outcome`].
///
/// # Errors
/// Returns [`crate::Error::Parse`] if `json` is not valid JSON or is missing a required field
/// (`total_cost_usd`, `usage`, `is_error`). `result` is no longer required — see the module's
/// leniency policy above; the `error_max_turns` envelope carries no `result` key at all.
pub fn parse_result(json: &str) -> Result<Outcome> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_usage(cost_usd: f64, output_tokens: u64) -> ModelUsage {
        ModelUsage {
            input_tokens: 0,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            cost_usd,
        }
    }

    fn outcome_with(models: &[(&str, f64, u64)]) -> Outcome {
        Outcome {
            cost_usd: 0.0,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            model_usage: models
                .iter()
                .map(|(name, cost, out)| ((*name).to_string(), model_usage(*cost, *out)))
                .collect(),
            text: "hi".to_string(),
            is_error: false,
            api_error_status: None,
            session_id: None,
            structured_output: None,
            subtype: None,
            num_turns: None,
            errors: Vec::new(),
        }
    }

    /// Runtime-inversion control for AC1: the pre-change shape (a required, non-defaulted `result`
    /// field) fails to deserialize the real `error_max_turns` fixture, immediately followed by
    /// the current, lenient `parse_result` succeeding on the exact same bytes.
    #[test]
    fn error_max_turns_envelope_would_have_failed_the_old_required_result_field() {
        /// Mirrors `Outcome::text`'s pre-change attribute: `result` required, no `default`.
        #[derive(Deserialize)]
        struct PreChangeShape {
            #[serde(rename = "result")]
            #[allow(dead_code)]
            text: String,
        }

        let fixture = include_str!("../tests/fixtures/error_max_turns.json");

        assert!(
            serde_json::from_str::<PreChangeShape>(fixture).is_err(),
            "the old required-result-field shape must reject the real error_max_turns envelope"
        );
        assert!(
            parse_result(fixture).is_ok(),
            "the current, lenient parse_result must accept the same envelope"
        );
    }

    #[test]
    fn error_max_turns_envelope_parses_subtype_num_turns_and_errors() {
        let fixture = include_str!("../tests/fixtures/error_max_turns.json");
        let outcome = parse_result(fixture).expect("the real error_max_turns envelope must parse");

        assert_eq!(outcome.subtype, Some(ResultSubtype::ErrorMaxTurns));
        assert_eq!(outcome.num_turns, Some(2));
        assert_eq!(
            outcome.errors,
            vec!["Reached maximum number of turns (1)".to_string()]
        );
        assert!(outcome.text.is_empty());
        assert!(outcome.session_id.is_some());
        assert!(outcome.cost_usd > 0.0);
    }

    #[test]
    fn unrecognised_subtype_parses_to_unknown() {
        let json = r#"{
            "total_cost_usd": 0.01,
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "result": "hi",
            "is_error": false,
            "subtype": "some_future_value"
        }"#;

        let outcome = parse_result(json).expect("an unrecognised subtype must not fail the parse");
        assert_eq!(
            outcome.subtype,
            Some(ResultSubtype::Unknown("some_future_value".to_string()))
        );
    }

    #[test]
    fn missing_cost_field_fails_parse() {
        let json = r#"{"usage": {"input_tokens": 1}, "result": "hi", "is_error": false}"#;
        assert!(parse_result(json).is_err());
    }

    #[test]
    fn unknown_fields_do_not_fail_parse() {
        let json = r#"{
            "total_cost_usd": 0.01,
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "result": "hi",
            "is_error": false,
            "some_future_field": {"nested": true}
        }"#;

        let outcome = parse_result(json).expect("an added vendor field must not break the parse");
        assert_eq!(outcome.text, "hi");
    }

    #[test]
    fn primary_model_picks_sole_entry() {
        let outcome = outcome_with(&[("claude-opus-4-8", 0.23, 4)]);
        assert_eq!(outcome.primary_model(), Some("claude-opus-4-8"));
    }

    #[test]
    fn primary_model_is_none_when_model_usage_is_empty() {
        let outcome = outcome_with(&[]);
        assert_eq!(
            outcome.primary_model(),
            None,
            "the error envelope reports `modelUsage: {{}}`"
        );
    }

    /// Cost leads token count: a chatty cheap model must not outrank the model that did the work.
    #[test]
    fn primary_model_ranks_by_cost_over_output_tokens() {
        let outcome = outcome_with(&[
            ("claude-haiku-4-5", 0.001, 900),
            ("claude-opus-4-8", 0.42, 12),
        ]);
        assert_eq!(outcome.primary_model(), Some("claude-opus-4-8"));
    }

    #[test]
    fn primary_model_falls_back_to_output_tokens_when_cost_ties() {
        let outcome = outcome_with(&[("model-a", 0.10, 5), ("model-b", 0.10, 50)]);
        assert_eq!(outcome.primary_model(), Some("model-b"));
    }

    /// Determinism: a total tie must resolve to the lexicographically first key, every run.
    #[test]
    fn primary_model_breaks_total_ties_lexicographically() {
        let outcome = outcome_with(&[("model-z", 0.10, 5), ("model-a", 0.10, 5)]);
        assert_eq!(outcome.primary_model(), Some("model-a"));
    }
}
