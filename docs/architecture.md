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
├── parse.rs      ← Outcome/Usage/ContentBlock + parse_result(), schema-locked to today's CLI JSON (implemented, CC.1.A)
└── isolation     ← CLAUDE_CONFIG_DIR temp dir + redacted credentials (stub: `pub mod isolation {}`, pending CC.1.B)
```

`config`, `execute`, and `parse` are now implemented as their own files (`CC.1.A`). `isolation`
remains an empty inline module in `lib.rs` pending `CC.1.B`.

## Core Types

- **`Error`** (`src/error.rs`) — crate-level error enum via `thiserror::Error`, covering
  `BinaryNotFound`, `Spawn(std::io::Error)`, `Timeout`, and `Parse(serde_json::Error)`. Re-exported
  from `lib.rs`.
- **`Result<T>`** (`src/error.rs`) — crate-wide alias `std::result::Result<T, Error>`, re-exported
  from `lib.rs`.
- **`Config`** (`src/config.rs`) — CLI invocation config: `system_prompt`, `append_system_prompt`,
  `model`, `allowed_tools`/`disallowed_tools`, `continue_session`/`resume`, plus unused `cwd`/`env`
  placeholder fields reserved for the `CC.1.B` isolation seam. `build_args(prompt)` builds the exact
  argv (always appending `--output-format json`). Re-exported from `lib.rs`.
- **`Outcome`** (`src/parse.rs`) — parsed CLI result: `cost_usd` (from `total_cost_usd`), `usage`
  (`Usage`), `model`, and `content` (`Vec<ContentBlock>`, defaulted). Re-exported from `lib.rs`.
- **`Usage`** (`src/parse.rs`) — token counts: `input_tokens`, `output_tokens`,
  `cache_creation_input_tokens`, `cache_read_input_tokens`.
- **`ContentBlock`** (`src/parse.rs`) — `Text { text }` or `Unknown { block_type, data }`; unrecognized
  block types fall through to `Unknown` for forward compatibility instead of failing the parse.

## Data Flow

Caller builds a `Config` → `execute(&config, prompt)` resolves the `claude` binary (`CLAUDE_BINARY`
env var, else `PATH` via `which`), spawns it with `config.build_args(prompt)` (env inherited from the
current process), wraps the whole call in one `tokio::time::timeout` (`kill_on_drop(true)` so a
timed-out/cancelled call never leaks a subprocess) → CLI emits `--output-format json` → `parse::parse_result`
extracts `total_cost_usd` + top-level `usage` + `model` (+ optional `content`) → `Outcome` returned to
the caller.
