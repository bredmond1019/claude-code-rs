---
type: Plan
title: Task Spec — Phase 1, Block A — execute core, inherit-env
description: Decomposed task spec for CC.1.A — the async execute() entry point over `claude -p`, its argv builder, and a schema-locked Outcome parser on today's CLI JSON.
doc_id: 1-a-execute-core-tasks
layer: [engine]
project: claude-code-rs
status: draft
keywords: [execute, subprocess, argv builder, parser, total_cost_usd, kill_on_drop, forward-compat]
related: [master-plan, status, planning-index, 0-a-foundation-setup-tasks]
---

# Task Spec — Phase 1, Block A — execute core, inherit-env

**Status:** Not started · **Last run:** never

## Goal
Ship `async fn execute(config: &Config, prompt: &str) -> Result<Outcome>` — the single subscription
transport entry point EN.2.A consumes — with an exact argv builder and a parser locked to today's
CLI JSON schema (`total_cost_usd` + top-level `usage` + `model`, forward-compat).

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 1 → Block A (`CC.1.A`). Read only that section.
- **Files (from the block):** New — `src/execute.rs`, `src/config.rs`, `src/parse.rs`,
  `tests/argv.rs`, `tests/parse_schema.rs`; Modified — `src/lib.rs` (replace the empty stub modules
  with real `pub mod` declarations + re-exports).
- **Out of scope (hard boundary):** token streaming, tiktoken offline counting, any API-credit HTTP
  path, and credential isolation — isolation is `CC.1.B`. Leave `pub mod isolation {}` as an empty
  stub; do not touch it.
- **Reference material (port, don't re-derive):**
  - Arg-builder shape: `../../portfolio/claude-sdk-rs/src/core/config.rs` (`-p`, `--system-prompt`,
    `--model`, `--allowedTools`/`--disallowedTools`, `--append-system-prompt`,
    `--continue`/`--resume`, `--output-format json`) — **drop** its security-validator/telemetry
    bloat and the `MAX_SYSTEM_PROMPT_LENGTH` "malicious content" checks.
  - Forward-compat deserialize (`Unknown` fallthrough variant): `#[serde(tag = "type")]` enum pattern
    in `../reference-repos/claude-agent-sdk-rust/src/types.rs` (`ContentBlock`).
- **Schema to lock (today's CLI, NOT the old shape):** parse `total_cost_usd` at the top level, the
  top-level `usage` object (`input_tokens`/`output_tokens`/`cache_*`), and `model` — NOT the stale
  `cost_usd`/`message.usage` shape that let `claude-sdk-rs` drift.
- **Standing rules (`CLAUDE.md`):** every block ships with tests (rule 1); gated checks are fmt ·
  clippy · test · build (`planning/harness.json`), all must stay green.
- **Current skeleton:** `src/lib.rs` declares `pub mod config {}` / `execute {}` / `parse {}` /
  `isolation {}` as empty inline stubs and re-exports `error::{Error, Result}`. `src/error.rs`
  already carries `BinaryNotFound`, `Spawn(io::Error)`, `Timeout`, `Parse(serde_json::Error)`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `src/config.rs` defines a `Config` struct covering the ported flags (system prompt, model,
  allowed/disallowed tools, append-system-prompt, continue/resume) plus an env/cwd seam placeholder,
  and a **public** argv builder (e.g. `Config::build_args(&self, prompt: &str) -> Vec<String>`) that
  always appends `-p` and `--output-format json`.
- `tests/argv.rs` asserts the **exact** argv vector built from a representative `Config` (flag order
  and values), locking the builder.
- `src/parse.rs` defines `Outcome` (cost from `total_cost_usd`, usage tokens from the top-level
  `usage`, `model`) and a parser over the CLI's JSON that reads **today's** schema, with an
  `Unknown` forward-compat variant so unrecognized content-block `type`s don't fail the parse.
- `tests/parse_schema.rs` feeds canned CLI JSON through the parser and asserts the expected `Outcome`
  (cost, usage, model), plus a case with an unknown content-block type that still parses — locking
  the schema.
- `src/execute.rs` defines `async fn execute(config: &Config, prompt: &str) -> Result<Outcome>` built
  on `tokio::process::Command` with `.kill_on_drop(true)` and a whole-call `tokio::time::timeout`
  (no per-line hardcoded timeout); binary resolution is `CLAUDE_BINARY` env → `which::which("claude")`,
  returning `Error::BinaryNotFound` when absent.
- `src/lib.rs` declares `pub mod config; pub mod execute; pub mod parse;` (real file modules) and
  re-exports `Config`, `Outcome`, and `execute`; `pub mod isolation {}` stays an empty stub.
- A **live** `#[ignore]`d smoke test exercises `execute` against a trivial prompt on the subscription
  and returns a populated `Outcome` — ignored so gated `cargo test` stays green; runnable manually
  with `cargo test -- --ignored`.
- All gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo build --release` — no warnings.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
_None yet._

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
