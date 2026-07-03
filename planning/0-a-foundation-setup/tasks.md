---
type: Plan
title: Task Spec — Phase 0, Block A — Foundation setup
description: Decomposed task spec for CC.0.A — scaffold the lean claude-code-rs crate skeleton and fill the Rust SDLC harness.
doc_id: 0-a-foundation-setup-tasks
layer: [engine, factory]
project: claude-code-rs
status: draft
keywords: [foundation, scaffold, cargo, crate skeleton, harness, thiserror]
related: [master-plan, status, planning-index]
---

# Task Spec — Phase 0, Block A — Foundation setup

**Status:** Done · **Last run:** 2026-07-03

## Goal
Scaffold the `claude-code-rs` crate — lean dep set, a warning-free module skeleton, a crate-level
`Error`/`Result`, and the Rust SDLC harness — as a clean, reproducible starting point before any
feature work.

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 0 → Block A (`CC.0.A`). Read only that section.
- **Files (from the block):** New/modified — `Cargo.toml`, `src/lib.rs`, `src/error.rs`,
  `planning/harness.json`, `.gitignore`.
- **Out of scope (hard boundary):** any subprocess execution, arg-building, or parsing logic —
  empty module stubs only. Those land in `CC.1.A` / `CC.1.B`.
- **Standing rules (`CLAUDE.md`):** every block ships with tests (rule 1); the run/test commands in
  `CLAUDE.md`'s *Build / test / run* section must match `planning/harness.json`'s `validation.checks[]`.
- **Harness profile:** the Rust (CLI/library, no web server) profile in
  `planning/harness.examples.md` — fmt · clippy · test · build, all gating; `uiTest.enabled: false`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `cargo build` succeeds with no warnings.
- `cargo test` succeeds with no warnings (at least one test present per standing rule 1).
- `cargo clippy -- -D warnings` is clean.
- `cargo fmt --check` passes.
- `src/lib.rs` declares the module skeleton — `config`, `execute`, `parse`, `isolation` (empty
  stubs) and `error` — and it compiles.
- `src/error.rs` defines a crate-level `Error` (via `thiserror`) and a `Result` alias, re-exported
  from `lib.rs`.
- `Cargo.toml` pins the lean dep set only: `tokio` (features `process`, `rt`, `macros`, `time` — not
  `full`), `serde`, `serde_json`, `thiserror`, `which`. No `async-trait`, `sqlx`, or telemetry deps.
- `planning/harness.json` holds the Rust profile (fmt/clippy/test/build, all `gates: true`;
  `stack: "rust"`; `uiTest.enabled: false`) with no `REPLACE-ME` sentinels.
- `CLAUDE.md`'s *Build / test / run* block lists the same commands as `planning/harness.json`.
- `.gitignore` ignores `/target`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- These are the same checks Task 4 writes into planning/harness.json (Rust profile). -->

## Notes
_None yet._

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
