//! Conformance tests: the parser against **real captured** `claude` CLI responses.
//!
//! These fixtures are ground truth — see `tests/fixtures/README.md` for provenance and the
//! re-capture procedure. Every assertion here is about what the vendor actually emitted, not what
//! this crate wishes it emitted.
//!
//! The predecessors of these tests were hand-written to match the parser and passed for months
//! while the real schema drifted out from under them. That is the failure this file exists to
//! prevent, and it is why nothing here may be authored by hand.

use std::collections::BTreeSet;

use claude_code_rs::parse::parse_result;

/// Real capture, `claude` 2.1.211, success envelope.
const FIXTURE_SUCCESS: &str = include_str!("fixtures/cli-result-2.1.211.json");

/// Real capture, `claude` 2.1.211, API error envelope (unroutable model, HTTP 404).
const FIXTURE_ERROR: &str = include_str!("fixtures/cli-error-2.1.211.json");

/// Real capture, `claude` 2.1.214, success envelope with `--json-schema` (structured output).
const FIXTURE_STRUCTURED: &str = include_str!("fixtures/cli-structured-2.1.214.json");

/// Top-level keys of an envelope.
fn top_level_keys(json: &str) -> BTreeSet<String> {
    let value: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
    value
        .as_object()
        .expect("envelope is a JSON object")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn parses_captured_success_envelope() {
    let outcome = parse_result(FIXTURE_SUCCESS).expect("real success capture must parse");

    assert_eq!(outcome.text, "hello", "response text comes from `result`");
    assert!(!outcome.is_error);
    assert_eq!(outcome.api_error_status, None);
    assert!(outcome.cost_usd > 0.0);
    assert_eq!(outcome.usage.output_tokens, 4);
    assert_eq!(outcome.usage.cache_read_input_tokens, 15031);
}

/// The model name exists **only** as a `modelUsage` key — there is no top-level `model` field.
/// This is the drift that broke EN.2.A's live test.
#[test]
fn model_name_is_read_from_the_model_usage_map() {
    let outcome = parse_result(FIXTURE_SUCCESS).expect("real success capture must parse");

    assert_eq!(outcome.primary_model(), Some("claude-opus-4-8"));

    let entry = outcome
        .model_usage
        .get("claude-opus-4-8")
        .expect("captured modelUsage entry");
    assert_eq!(entry.output_tokens, 4);
    assert!(entry.cost_usd > 0.0, "per-model costUSD must deserialize");
}

/// There is no top-level `model` or `content` in a real response. Both were fabricated by the
/// fixtures this file replaced. Pin their absence so nobody reintroduces them.
#[test]
fn captured_envelope_has_no_top_level_model_or_content() {
    let keys = top_level_keys(FIXTURE_SUCCESS);

    assert!(
        !keys.contains("model"),
        "the CLI reports models via `modelUsage`, never a top-level `model`"
    );
    assert!(
        !keys.contains("content"),
        "the CLI reports text via `result`, never top-level `content` blocks"
    );
}

/// The error envelope shares `result`, so it parses cleanly — `execute()` is what turns `is_error`
/// into an [`claude_code_rs::Error::Api`]. Parsing must not itself fail here.
#[test]
fn parses_captured_error_envelope() {
    let outcome = parse_result(FIXTURE_ERROR).expect("real error capture must parse");

    assert!(outcome.is_error);
    assert_eq!(outcome.api_error_status, Some(404));
    assert!(
        outcome.text.contains("does-not-exist-xyz"),
        "`result` carries the error message on the error envelope"
    );
    assert!(
        outcome.model_usage.is_empty(),
        "the error envelope reports `modelUsage: {{}}`"
    );
    assert_eq!(
        outcome.primary_model(),
        None,
        "no model ran, so there is no primary model to name"
    );
}

/// Pins the trap: the error envelope self-reports `subtype: "success"`. Anything branching on
/// `subtype` to detect failure is wrong. If a future capture stops proving this, `execute()`'s
/// dispatch should be revisited — not this assertion loosened.
#[test]
fn captured_error_envelope_still_claims_subtype_success() {
    let raw: serde_json::Value =
        serde_json::from_str(FIXTURE_ERROR).expect("fixture is valid JSON");

    assert_eq!(raw["subtype"], "success");
    assert_eq!(raw["is_error"], true);
}

/// A `--json-schema` request populates `structured_output`, and `result` still carries the same
/// object as a JSON string — the parser must agree with itself on both representations.
#[test]
fn parses_captured_structured_output_envelope() {
    let outcome = parse_result(FIXTURE_STRUCTURED).expect("real structured capture must parse");

    let expected = serde_json::json!({
        "city": "Paris",
        "population": 2100000
    });

    assert_eq!(
        outcome.structured_output,
        Some(expected.clone()),
        "structured_output must deserialize the CLI's `structured_output` object"
    );
    assert_eq!(
        outcome.text,
        expected.to_string(),
        "`result` carries the same object as its JSON string form"
    );
}

/// A schemaless call never sends `--json-schema`, so the CLI never emits the `structured_output`
/// key at all. Pin that this defaults cleanly to `None` rather than failing to parse.
#[test]
fn parses_captured_success_envelope_without_structured_output() {
    let outcome = parse_result(FIXTURE_SUCCESS).expect("real success capture must parse");

    assert_eq!(
        outcome.structured_output, None,
        "a schemaless capture must default structured_output to None"
    );
}

/// Documents the success/error envelope difference, so the canary below can compare like with like.
#[test]
fn error_envelope_is_a_subset_of_the_success_envelope() {
    let success = top_level_keys(FIXTURE_SUCCESS);
    let error = top_level_keys(FIXTURE_ERROR);

    let only_in_error: Vec<_> = error.difference(&success).collect();
    assert!(
        only_in_error.is_empty(),
        "error envelope gained keys the success envelope lacks: {only_in_error:?}"
    );

    let only_in_success: Vec<_> = success.difference(&error).map(String::as_str).collect();
    assert_eq!(
        only_in_success,
        vec!["time_to_request_ms", "ttft_ms", "ttft_stream_ms"],
        "only the streaming-timing keys should be success-only"
    );
}

/// **The drift canary.** Runs the live CLI and diffs its top-level key set against the captured
/// success fixture.
///
/// This compares live output to *the fixture* — never to a hand-written list. A "known list" is a
/// hand-written fixture with extra steps and would rot exactly the way the old tests did.
///
/// Both directions fail, deliberately. A missing key is the bug class of 2026-07-16; an extra key
/// means the fixture is stale and the parser is reasoning about an obsolete shape. A warning inside
/// an `#[ignore]`d test nobody runs is worth zero, and re-capturing is a two-minute chore.
///
/// Run with `cargo test -- --ignored` after any CLI upgrade.
#[tokio::test]
#[ignore]
async fn live_response_key_set_matches_captured_fixture() {
    let raw = tokio::process::Command::new("claude")
        .args(["-p", "Reply with exactly: hello", "--output-format", "json"])
        .output()
        .await
        .expect("live CLI call");

    let live = String::from_utf8_lossy(&raw.stdout);
    assert!(
        !live.trim().is_empty(),
        "live CLI produced no stdout: {}",
        String::from_utf8_lossy(&raw.stderr)
    );

    let live_keys = top_level_keys(&live);
    let fixture_keys = top_level_keys(FIXTURE_SUCCESS);

    let missing: Vec<_> = fixture_keys.difference(&live_keys).collect();
    let added: Vec<_> = live_keys.difference(&fixture_keys).collect();

    assert!(
        missing.is_empty(),
        "the CLI DROPPED top-level keys {missing:?} — the parser may be reading fields that no \
         longer exist. This is the 2026-07-16 bug class. Fix the parser, then re-capture."
    );
    assert!(
        added.is_empty(),
        "additive drift: the CLI now emits {added:?}. Re-capture the fixture per \
         tests/fixtures/README.md, then update FIXTURE_SUCCESS."
    );

    parse_result(&live).expect("a live response must parse with the current parser");
}

/// **Structured-output canary.** Runs the live CLI with `--json-schema` and confirms
/// `structured_output` is actually populated, not just parseable.
///
/// This is a narrower check than [`live_response_key_set_matches_captured_fixture`] — it does not
/// diff the full key set, because `--json-schema` is opt-in and would otherwise force every
/// caller to keep two full-envelope fixtures in lockstep. It exists to catch the CLI silently
/// dropping or renaming `structured_output` without anyone noticing.
///
/// Run with `cargo test -- --ignored` after any CLI upgrade.
#[tokio::test]
#[ignore]
async fn live_structured_output_is_populated() {
    let raw = tokio::process::Command::new("claude")
        .args([
            "-p",
            "Return the capital of France and its approximate population.",
            "--output-format",
            "json",
            "--json-schema",
            r#"{"type":"object","properties":{"city":{"type":"string"},"population":{"type":"integer"}},"required":["city","population"]}"#,
        ])
        .output()
        .await
        .expect("live CLI call");

    let live = String::from_utf8_lossy(&raw.stdout);
    assert!(
        !live.trim().is_empty(),
        "live CLI produced no stdout: {}",
        String::from_utf8_lossy(&raw.stderr)
    );

    let outcome = parse_result(&live).expect("a live structured response must parse");
    assert!(
        outcome.structured_output.is_some(),
        "a --json-schema call must populate structured_output"
    );
}
