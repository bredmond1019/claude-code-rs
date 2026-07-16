# CLI response fixtures — provenance

These are **real, captured** `claude` CLI responses. They are the ground truth this crate's parser
is tested against. Nothing here is hand-written, and nothing here may be hand-edited.

> **Why this file exists.** Before 2026-07-16 every fixture in this crate was hand-authored to match
> `parse.rs` rather than captured from the CLI. The tests therefore asserted the parser agreed with
> itself, and passed for months while the real schema drifted out from under them — `model` moved
> into `modelUsage`, and response text moved from `content` blocks to `result`. Six documents and
> test suites all agreed with each other and all were wrong. A captured fixture is the only artifact
> that can't lie about the vendor's shape. See `planning/decisions/D2-cli-schema-provenance.md`.

## What's here

| File | Captured from | Envelope |
|---|---|---|
| `cli-result-2.1.211.json` | `claude` 2.1.211, 2026-07-16 | success (`is_error: false`) |
| `cli-error-2.1.211.json` | `claude` 2.1.211, 2026-07-16 | API error (`is_error: true`, HTTP 404) |

The filename carries the CLI version. **That is the version record** — this crate deliberately has
no contract doc, changelog, or semver for the CLI schema: the other party is a vendor who never
agreed to any of it. The filename is the version, the tests are the enforcement, this file is the
provenance.

A fixture that was *not* captured must be named `*-HANDWRITTEN.json`. Never let an authored fixture
wear a version number — that is precisely the lie described above.

## Capture procedure

```bash
# success envelope
claude -p "Reply with exactly: hello" --output-format json > cli-result-<version>.json

# error envelope (an unroutable model is the cheapest reliable trigger)
claude -p "hi" --model does-not-exist-xyz --output-format json > cli-error-<version>.json
```

Then redact **only** the identifying fields, replacing each with the all-zero sentinel
`00000000-0000-0000-0000-000000000000`:

- `session_id`
- `uuid`

**Redact nothing else.** `duration_ms`, `ttft_ms`, `num_turns`, `total_cost_usd`, `usage`, and
`modelUsage` are non-deterministic, and that is fine — the conformance tests assert *structure and
types*, never values. Scrubbing them would destroy the provenance that makes these files worth
having.

## Re-capture procedure

The `#[ignore]`d canary test (`tests/parse_schema.rs`) diffs a live CLI response's top-level key set
against `cli-result-*.json`. Run it whenever the CLI is upgraded:

```bash
cargo test -- --ignored
```

If it fails:

1. **Missing key** — the vendor removed a field. This is the bug class of 2026-07-16. Fix the parser.
2. **Extra key** — additive drift. Re-run the capture above, re-redact, name the file for the new CLI
   version, delete the old one, and update `FIXTURE_SUCCESS` in `tests/parse_schema.rs`.

Both are failures, not warnings. A warning inside an `#[ignore]`d test nobody runs is worth zero.

## What we depend on, and what we ignore

Everything the parser reads, and — more importantly — everything it deliberately does not. Without
this table, the next person "helpfully" parses `subtype` and reintroduces the bug.

| Field | Status | Notes |
|---|---|---|
| `total_cost_usd` | **depend** | → `Outcome::cost_usd`. |
| `usage` | **depend** | → `Outcome::usage`. Carries six further fields (`server_tool_use`, `service_tier`, `cache_creation`, `inference_geo`, `iterations`, `speed`) that we ignore. |
| `modelUsage` | **depend** | → `Outcome::model_usage`. Object keyed by model name; `{}` on the error envelope. The only place the model name appears. |
| `result` | **depend** | → `Outcome::text`. Present on **both** envelopes — carries the reply on success and the error message on failure. |
| `is_error` | **depend** | **The only trustworthy error signal.** |
| `api_error_status` | **depend** | `null` on success, HTTP status on error. |
| `subtype` | **IGNORE — it lies** | Reports `"success"` even when `is_error: true`. Do not use it to detect errors. |
| `type` | ignore | Always `"result"` for `--output-format json`. |
| `stop_reason`, `terminal_reason` | ignore | `terminal_reason` does track errors (`"api_error"`), but `is_error` is simpler and sufficient. |
| `session_id`, `uuid` | ignore | Identifying; redacted here. |
| `duration_ms`, `duration_api_ms`, `ttft_ms`, `ttft_stream_ms`, `time_to_request_ms` | ignore | Timing. The three `ttft`/`time_to_request` keys are **absent from the error envelope** — which is why the canary compares success-to-success only. |
| `num_turns`, `permission_denials`, `fast_mode_state` | ignore | Not needed by any consumer today. |

## Envelope differences

The error envelope is a strict subset of the success envelope: 17 shared top-level keys, with
`ttft_ms`, `ttft_stream_ms`, and `time_to_request_ms` present only on success. Any parser field
required on one path must therefore be satisfiable on the other — which is why `text` (`result`) can
be a required `String` while `api_error_status` is an `Option`.
