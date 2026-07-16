---
type: Reference
title: claude-code-rs Architecture
description: Module map, core types, and data flow for the subscription Claude Code subprocess SDK.
doc_id: architecture
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [architecture, subprocess, tokio, credential-isolation, module-map]
related: [api]
---

# claude-code-rs — Architecture

## Overview

A lean async Rust SDK that drives the `claude` CLI as a subprocess (`claude -p`) on the flat-rate
subscription rather than Anthropic API credits. One placeholder line per section below — `/document`
and `/update-docs --bootstrap` fill these in as blocks ship.

## Module Map

```
src/
├── lib.rs        ← crate root; re-exports Config/Error/Result/execute/Outcome; declares module skeleton
├── error.rs      ← thiserror crate-level Error enum + Result<T> alias (implemented, CC.0.A)
├── config.rs     ← Config struct + build_args() CLI arg-builder (implemented, CC.1.A)
├── execute.rs    ← async execute(): binary resolution, spawn, whole-call timeout, kill_on_drop (implemented, CC.1.A)
├── parse.rs      ← Outcome/Usage/ModelUsage + parse_result(); shape defined by tests/fixtures/ (implemented, CC.1.A)
└── isolation.rs  ← IsolatedConfigDir RAII guard: temp CLAUDE_CONFIG_DIR with redacted credentials (implemented, CC.1.B)
```

`config`, `execute`, `parse`, and `isolation` are all implemented as their own files.
`execute()` applies `Config::cwd`/`Config::env` and, when `Config::isolated` is `true`, builds an
`IsolatedConfigDir` and sets `CLAUDE_CONFIG_DIR` for the child process (`CC.1.B`).

## Core Types

- **`Error`** (`src/error.rs`) — crate-level error enum via `thiserror::Error`, covering
  `BinaryNotFound`, `Spawn(std::io::Error)`, `Timeout`, `Parse(serde_json::Error)`,
  `Cli { status, stderr }` (the CLI itself failed), `Api { status, message }` (the CLI ran, the API
  call failed), and `Isolation(std::io::Error)` (temp dir creation or a credentials/`.claude.json` source that exists
  but could not be read/copied). Re-exported from `lib.rs`.
- **`Result<T>`** (`src/error.rs`) — crate-wide alias `std::result::Result<T, Error>`, re-exported
  from `lib.rs`.
- **`Config`** (`src/config.rs`) — CLI invocation config: `system_prompt`, `append_system_prompt`,
  `model`, `allowed_tools`/`disallowed_tools`, `continue_session`/`resume`, plus `cwd`/`env` overrides
  (now applied by `execute()` via `Command::current_dir`/`Command::envs`) and the opt-in `isolated: bool`
  switch (default `false`) that routes the call through `IsolatedConfigDir`. `build_args(prompt)`
  builds the exact argv (always appending `--output-format json`). Re-exported from `lib.rs`.
- **`IsolatedConfigDir`** (`src/isolation.rs`) — RAII guard that builds a throwaway
  `CLAUDE_CONFIG_DIR` containing a `refreshToken`-redacted copy of `.credentials.json` (mode `0600`,
  sourced from the macOS Keychain then `~/.claude/.credentials.json` fallback) and an optional copy
  of `.claude.json`; removes the temp dir (including on mid-construction failure) on drop.
  `IsolatedConfigDir::new()` uses the real credential sources; `with_sources(creds_json,
  claude_json_src)` is an injectable constructor for tests. Re-exported from `lib.rs`.
- **`Outcome`** (`src/parse.rs`) — parsed CLI result: `cost_usd` (from `total_cost_usd`), `usage`
  (`Usage`), `model_usage` (`BTreeMap<String, ModelUsage>`, from `modelUsage`), `text` (from
  `result`), `is_error`, and `api_error_status`. There is **no** top-level `model` field — the model
  name exists only as a `model_usage` key; `Outcome::primary_model()` picks one by a documented
  heuristic (cost, then output tokens, then key order) and returns `None` when none ran.
  Re-exported from `lib.rs`. **The authority for this shape is `tests/fixtures/`** — real captured
  CLI responses — per decision D2, not this page.
- **`Usage`** (`src/parse.rs`) — token counts: `input_tokens`, `output_tokens`,
  `cache_creation_input_tokens`, `cache_read_input_tokens`.
- **`ModelUsage`** (`src/parse.rs`) — per-model counts plus `cost_usd` (from `costUSD`). Note the CLI
  emits these keys in camelCase, unlike the snake_case top-level `usage`.

## Data Flow

Caller builds a `Config` → `execute(&config, prompt)` resolves the `claude` binary (`CLAUDE_BINARY`
env var, else `PATH` via `which`), applies `config.cwd` (`Command::current_dir`) and `config.env`
(`Command::envs`, on top of the inherited environment); when `config.isolated` is `true`, builds an
`IsolatedConfigDir` guard first (surfacing `Error::Isolation` before ever spawning the child) and sets
`CLAUDE_CONFIG_DIR` in the child env, keeping the guard alive until after the child's output is
read — spawns it with `config.build_args(prompt)`, wraps the whole call in one
`tokio::time::timeout` (`kill_on_drop(true)` so a timed-out/cancelled call never leaks a subprocess)
→ CLI emits `--output-format json` → `parse::parse_result` extracts `total_cost_usd`, top-level
`usage`, `modelUsage`, and `result` → `Outcome` returned to the caller. The default (non-isolated,
no overrides) path is unchanged.

Failure routing (two distinct modes, verified against CLI 2.1.211 — the exit code alone cannot
distinguish them, since both exit non-zero):

- **CLI failure** (bad argv, missing prompt): stdout is empty, the message is on stderr →
  `Error::Cli { status, stderr }`.
- **API failure** (unroutable model, outage): stdout carries a well-formed envelope with
  `is_error: true` and **stderr is empty** — the message is in the JSON's `result` field →
  `Error::Api { status, message }`.

`execute()` therefore dispatches on whether stdout is empty, then on `is_error`. It must never
branch on the envelope's `subtype`, which reports `"success"` on both paths.
