//! Canned-JSON schema lock test for `claude_code_rs::parse`.
//!
//! Feeds canned `claude` CLI JSON (today's schema: top-level `total_cost_usd`, top-level
//! `usage`, `model`) through the public parser and asserts the resulting `Outcome` — locking
//! the schema so a future CLI drift is caught here. Also asserts that a response carrying an
//! unrecognized content-block `type` still parses successfully (forward-compat `Unknown`
//! variant), rather than erroring out.

use claude_code_rs::parse::{parse_result, ContentBlock};

#[test]
fn parses_canned_cli_json_into_expected_outcome() {
    let json = r#"{
        "total_cost_usd": 0.1234,
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 5,
            "cache_read_input_tokens": 7
        },
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "hello from the CLI"}]
    }"#;

    let outcome = parse_result(json).expect("canned CLI JSON should parse");

    assert_eq!(outcome.cost_usd, 0.1234);
    assert_eq!(outcome.usage.input_tokens, 100);
    assert_eq!(outcome.usage.output_tokens, 50);
    assert_eq!(outcome.usage.cache_creation_input_tokens, 5);
    assert_eq!(outcome.usage.cache_read_input_tokens, 7);
    assert_eq!(outcome.model, "claude-sonnet-4-5");
    assert_eq!(
        outcome.content,
        vec![ContentBlock::Text {
            text: "hello from the CLI".to_string()
        }]
    );
}

#[test]
fn unknown_content_block_type_still_parses() {
    let json = r#"{
        "total_cost_usd": 0.02,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 8,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0
        },
        "model": "claude-sonnet-4-5",
        "content": [
            {"type": "text", "text": "known block"},
            {"type": "tool_use_v2", "id": "abc123", "payload": {"nested": true}}
        ]
    }"#;

    let outcome =
        parse_result(json).expect("response with an unknown content-block type should still parse");

    assert_eq!(outcome.content.len(), 2);
    assert_eq!(
        outcome.content[0],
        ContentBlock::Text {
            text: "known block".to_string()
        }
    );

    match &outcome.content[1] {
        ContentBlock::Unknown { block_type, data } => {
            assert_eq!(block_type, "tool_use_v2");
            assert_eq!(data.get("id").and_then(|v| v.as_str()), Some("abc123"));
        }
        other => panic!("expected Unknown variant for unrecognized block type, got {other:?}"),
    }
}
