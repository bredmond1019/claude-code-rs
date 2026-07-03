//! Outcome type and schema-locked parser for the `claude` CLI's JSON output.
//!
//! Locks onto **today's** CLI schema — `total_cost_usd` at the top level, the
//! top-level `usage` object, and `model` — not the stale `cost_usd`/`message.usage`
//! shape from earlier CLI versions. Any content blocks carried on the response are
//! parsed leniently: an unrecognized block `type` is captured in
//! [`ContentBlock::Unknown`] instead of failing the parse (forward compatibility).

use serde::{Deserialize, Deserializer};

use crate::error::Result;

/// Token usage reported by the `claude` CLI for a single call.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Usage {
    /// Tokens consumed by the input (prompt + context).
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

/// A single content block carried on a `claude` CLI response.
///
/// Unrecognized block `type`s (e.g. new CLI features) are captured in
/// [`ContentBlock::Unknown`] instead of causing a deserialization error, so this
/// SDK stays forward-compatible with CLI releases that add block types.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    /// Plain text content.
    Text {
        /// The text itself.
        text: String,
    },

    /// A block whose `type` this SDK does not (yet) recognize.
    Unknown {
        /// The raw `type` field value.
        block_type: String,
        /// The full raw JSON of the block.
        data: serde_json::Value,
    },
}

/// Helper enum mirroring the known [`ContentBlock`] variants, with derived
/// `Deserialize`. Used by `ContentBlock`'s custom `Deserialize` impl so that a
/// failed match falls through to `Unknown` instead of erroring.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockHelper {
    Text { text: String },
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;

        match serde_json::from_value::<ContentBlockHelper>(value.clone()) {
            Ok(ContentBlockHelper::Text { text }) => Ok(ContentBlock::Text { text }),
            Err(_) => {
                let block_type = value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                Ok(ContentBlock::Unknown {
                    block_type,
                    data: value,
                })
            }
        }
    }
}

/// Parsed result of a single `claude` CLI invocation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Outcome {
    /// Total cost of the call in USD (`total_cost_usd`).
    #[serde(rename = "total_cost_usd")]
    pub cost_usd: f64,

    /// Token usage for the call (top-level `usage` object).
    pub usage: Usage,

    /// The model that produced the response.
    pub model: String,

    /// Content blocks carried on the response, if any. Unrecognized block types
    /// are preserved as [`ContentBlock::Unknown`] rather than failing the parse.
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

/// Parse a `claude` CLI JSON response into an [`Outcome`].
///
/// # Errors
/// Returns [`crate::Error::Parse`] if `json` is not valid JSON or is missing a
/// required field (`total_cost_usd`, `usage`, or `model`).
pub fn parse_result(json: &str) -> Result<Outcome> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_today_schema() {
        let json = r#"{
            "total_cost_usd": 0.045,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_creation_input_tokens": 1,
                "cache_read_input_tokens": 2
            },
            "model": "claude-sonnet-4-5",
            "content": [{"type": "text", "text": "hi"}]
        }"#;

        let outcome = parse_result(json).expect("parse should succeed");
        assert_eq!(outcome.cost_usd, 0.045);
        assert_eq!(outcome.usage.input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 20);
        assert_eq!(outcome.model, "claude-sonnet-4-5");
        assert_eq!(
            outcome.content,
            vec![ContentBlock::Text {
                text: "hi".to_string()
            }]
        );
    }

    #[test]
    fn unknown_content_block_type_does_not_fail_parse() {
        let json = r#"{
            "total_cost_usd": 0.01,
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "model": "claude-sonnet-4-5",
            "content": [{"type": "some_future_block", "foo": "bar"}]
        }"#;

        let outcome = parse_result(json).expect("unknown block type should not fail parse");
        assert_eq!(outcome.content.len(), 1);
        match &outcome.content[0] {
            ContentBlock::Unknown { block_type, .. } => {
                assert_eq!(block_type, "some_future_block");
            }
            other => panic!("expected Unknown variant, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_fails_parse() {
        let json = r#"{"usage": {"input_tokens": 1, "output_tokens": 1}, "model": "x"}"#;
        assert!(parse_result(json).is_err());
    }
}
