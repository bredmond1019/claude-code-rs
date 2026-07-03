---
type: Plan
title: claude-code-rs Master Plan
description: Strategic roadmap and phase specifications for claude-code-rs — the subscription Claude Code subprocess SDK.
doc_id: master-plan
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [master plan, roadmap, phases, blocks, subprocess, subscription, credential-isolation]
related: [context, status, planning-index]
---

# claude-code-rs — Master Plan

*Living document. Created 2026-07-03. Source: `agentic-portfolio/planning/claude-code-rs/plan.md`.*

## The Goal, Stated Plainly

A lean, async Rust SDK that runs the `claude` CLI as a subprocess (`claude -p`) so callers can drive
Claude Code programmatically **on Brandon's flat-rate Claude Code subscription, not Anthropic API
credits**. Simply spawning `claude -p` while inheriting the environment already authenticates on the
logged-in subscription — no API credits are consumed. The crate exists to make that reliable and
safe: a clean argv-builder over `Config`, a schema-locked result parser, and a credential-isolation
seam so a background subprocess cannot log out Brandon's concurrent interactive session.

"Ready" for Milestone 1 means `engine-rs`'s `ClaudeCodeStep` node (EN.2.A) can depend on this crate
via a Cargo path dep (`../claude-code-rs`), call `execute`, and receive a parsed `Outcome` carrying
cost and usage — with concurrent isolated sessions proven safe by test.

## The Destination

The subscription transport substrate for the `core/` engine. First consumer: `engine-rs` EN.2.A
(recorded in engine-rs decision D4). Built fresh rather than repairing `portfolio/claude-sdk-rs`
v2.0.0 — that crate's reusable core is small (the CLI arg-builder), its cost parser reads the stale
`cost_usd`/`message.usage` schema instead of today's `total_cost_usd` + top-level `usage`, it lacks
`kill_on_drop` (orphaned `claude` processes) and any `CLAUDE_CONFIG_DIR` seam, and its `2.0.0` semver
docs no longer match. `claude-sdk-rs` stays the v1 portfolio artifact; this crate is the lean,
correct successor living in `core/` alongside the rest of the engine substrate.

## Architecture / Design Overview

```
caller (engine-rs ClaudeCodeStep)
        │  Config + prompt
        ▼
  execute()  ──build argv──►  tokio::process::Command  ──spawn──►  `claude -p …`
        │                        .kill_on_drop(true)                    │
        │                        whole-call tokio timeout                │  --output-format json
        │                        env: inherited OR isolated              ▼
        │                                                        stream-json / result
        └──────────────── Outcome ◄──── parse: total_cost_usd + top-level usage + model
                                              (Unknown-variant forward-compat deserialize)

  isolation module (optional env seam):
    temp CLAUDE_CONFIG_DIR ← .credentials.json (chmod 0600, refreshToken DELETED, from Keychain)
                           ← copy of ~/.claude.json     → rmtree on drop
```

Load-bearing decisions: (1) inherit-env by default — auth is free; (2) `kill_on_drop(true)` +
whole-call timeout instead of the old per-line 30s timeout; (3) parse **today's** CLI JSON schema and
lock it with canned-JSON tests; (4) isolation redacts `refreshToken` so a subprocess token-refresh
cannot invalidate the interactive session. Deps stay lean: `tokio` (process/rt/macros/time — not
`full`), `serde`/`serde_json`, `thiserror`, `which`. No `async-trait`, no `sqlx`, no telemetry.

---

## Phase 0 — Foundation

### Block A — Foundation setup (`CC.0.A`)
- **What:** Scaffold the crate: `cargo init --lib`, add the lean dep set (`tokio` with only
  `process`/`rt`/`macros`/`time`, `serde`/`serde_json`, `thiserror`, `which`), a `lib.rs` that
  declares the module skeleton (`config`, `execute`, `parse`, `isolation`, `error`), a crate-level
  `Error` (thiserror) and `Result` alias, and fill `planning/harness.json` with the Rust profile.
- **Why:** Establish a clean, reproducible, warning-free starting point before feature work; give the
  SDLC pipeline a real validation suite to run.
- **Files:** `Cargo.toml`, `src/lib.rs`, `src/error.rs`, `planning/harness.json`, `.gitignore`.
- **Out of scope:** Any subprocess execution, arg-building, or parsing logic (empty module stubs only).
- **Acceptance criteria:** `cargo build` and `cargo test` succeed with no warnings; `cargo clippy`
  is clean; the module skeleton compiles; the run/test commands in `CLAUDE.md` match
  `planning/harness.json`.

---

## Phase 1 — Milestone 1: usable subscription transport

### Block A — `execute` core, inherit-env (`CC.1.A`)
- **What:** `async fn execute(config: &Config, prompt: &str) -> Result<Outcome>` built on
  `tokio::process::Command` with `.kill_on_drop(true)` and a whole-call `tokio::time::timeout` (no
  per-line hardcoded timeout). Binary resolution: `CLAUDE_BINARY` env → `which::which("claude")`.
  Port the good arg-builder from `claude-sdk-rs` (`-p`, `--system-prompt`, `--model`,
  `--allowedTools`/`--disallowedTools`, `--append-system-prompt`, `--continue`/`--resume`,
  `--output-format json`), dropping the security-validator/telemetry bloat. Parser reads
  `total_cost_usd` (top level) + the top-level `usage` object (`input_tokens`/`output_tokens`/
  `cache_*`) + `model` — NOT the old `cost_usd`/`message.usage` shape — with an `Unknown`-variant
  forward-compat deserialize (copied from `core/reference-repos/claude-agent-sdk-rust/src/types.rs`)
  so future CLI content-block additions don't break parsing.
- **Why:** This is the single entry point EN.2.A consumes; inheriting the environment authenticates on
  the subscription with zero API credits. Locking the parser to today's schema closes the exact gap
  that let `claude-sdk-rs` silently drift.
- **Files:** `src/execute.rs`, `src/config.rs`, `src/parse.rs`, `src/lib.rs` (re-exports),
  `tests/argv.rs`, `tests/parse_schema.rs`.
- **Out of scope:** Token streaming, tiktoken offline counting, any API-credit HTTP path, credential
  isolation (that is `CC.1.B`).
- **Acceptance criteria:** A test asserts the exact argv built from a representative `Config`; canned
  CLI JSON fed through the parser yields the expected `Outcome` (cost, usage, model) and locks the
  schema; a live `execute` against a trivial prompt returns a populated `Outcome` on the subscription.

### Block B — Credential isolation (`CC.1.B`)
- **What:** An `isolation` module plus a `Config` env/cwd override seam. On spawn: build a temp
  `CLAUDE_CONFIG_DIR`; write `.credentials.json` (chmod `0600`, with `claudeAiOauth.refreshToken`
  **deleted**) sourced from the macOS Keychain (`security find-generic-password -a <USER> -w -s
  "Claude Code-credentials"`; best-effort, falling back to file-based `~/.claude/.credentials.json`
  on non-macOS); copy `~/.claude.json`; set `CLAUDE_CONFIG_DIR` in the child env; `rmtree` on
  drop/exit. Port the trick from
  `core/reference-repos/claude-agent-sdk-python/src/claude_agent_sdk/_internal/session_resume.py`
  (`_read_keychain_credentials`, `_copy_auth_files`, `_write_redacted_credentials`).
- **Why:** Brandon runs interactive Claude **and** subprocess Claude simultaneously. Without redacting
  `refreshToken`, a subprocess token-refresh can invalidate — and log out — the interactive session.
  This makes concurrent isolated sessions safe.
- **Files:** `src/isolation.rs`, `src/config.rs` (env/cwd seam), `src/execute.rs` (wire the seam),
  `tests/isolation.rs`.
- **Out of scope:** Non-macOS Keychain equivalents beyond the file-based fallback; any change to the
  inherit-env default path (isolation is opt-in via `Config`).
- **Acceptance criteria:** A test asserts the temp-dir layout is correct and that the written
  `.credentials.json` contains no `refreshToken`; an isolated `execute` runs concurrently with an
  interactive session without logging it out.

---

## Phase 2 — Depth / Hardening (forward-looking, out of Milestone 1)

### Block A — Streaming output (`CC.2.A`)
- **What:** A streaming variant of `execute` that yields live token/tool events by adapting the
  reference SDK's `eventsource`-style loop to the CLI's newline-delimited JSON (`stream-json`).
- **Why:** A consumer (e.g. a `bastion` surface) may need live token/tool events rather than a single
  final `Outcome`.
- **Files:** `src/stream.rs`, `src/lib.rs` (re-export), `tests/stream.rs`.
- **Out of scope:** Building any UI; changing the non-streaming `execute` contract.
- **Acceptance criteria:** A streaming call over canned newline-delimited JSON emits the expected
  ordered sequence of events and a terminal `Outcome`; back-pressure and cancellation drop the child
  via `kill_on_drop`.

### Block B — Multi-turn conversation helper (`CC.2.B`)
- **What:** A `ConversationBuilder`-style multi-turn helper (borrow the shape from
  `core/reference-repos/claude-agent-sdk-rust`) layered over `--continue`/`--resume` for state
  management beyond single calls.
- **Why:** Multi-turn workflows need session continuity without the caller hand-threading
  `--resume` session IDs.
- **Files:** `src/conversation.rs`, `src/lib.rs` (re-export), `tests/conversation.rs`.
- **Out of scope:** Persistent session storage (no sqlite/`SessionManager` — that was the
  `claude-sdk-rs` bloat this crate deliberately avoids).
- **Acceptance criteria:** A multi-turn conversation preserves session continuity across turns via
  `--resume`; a test asserts the correct resume/continue flags are threaded between turns.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A (`CC.0.A`) | Foundation setup — lean crate skeleton + harness | Clean, reproducible start | Enables everything downstream |
| 1 | A (`CC.1.A`) | `execute` core, inherit-env + schema-locked parser | Zero-credit subscription transport | The entry point EN.2.A consumes |
| 1 | B (`CC.1.B`) | Credential isolation (`CLAUDE_CONFIG_DIR` + redacted refreshToken) | Concurrent sessions can't log each other out | Makes background use safe |
| 2 | A (`CC.2.A`) | Streaming output | Live token/tool events | Optional richer consumer surface |
| 2 | B (`CC.2.B`) | Multi-turn conversation helper | Session continuity | Multi-turn ergonomics |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up where you
left off.*
