---
type: Reference
title: claude-code-rs Architecture
description: Module map, core types, and data flow for the subscription Claude Code subprocess SDK.
doc_id: architecture
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [architecture, subprocess, tokio, credential-isolation, module-map]
related: [api, claude-code-rs]
---

# claude-code-rs — Architecture

## Overview

A lean async Rust SDK that drives the `claude` CLI as a subprocess (`claude -p`) on the flat-rate
subscription rather than Anthropic API credits. One placeholder line per section below — `/document`
and `/update-docs --bootstrap` fill these in as blocks ship.

## Module Map

```
src/
├── lib.rs        ← crate root; re-exports Error/Result; declares the module skeleton below
├── error.rs      ← thiserror crate-level Error enum + Result<T> alias (implemented, CC.0.A)
└── (inline stub modules declared in lib.rs, empty pending later blocks)
    ├── config     ← Config → CLI arg-builder (stub: `pub mod config {}`)
    ├── execute    ← async execute(): spawn, timeout, kill_on_drop (stub: `pub mod execute {}`)
    ├── parse      ← CLI stream-json / result parsing, forward-compat deserialize (stub: `pub mod parse {}`)
    └── isolation  ← CLAUDE_CONFIG_DIR temp dir + redacted credentials (stub: `pub mod isolation {}`)
```

`config`, `execute`, `parse`, and `isolation` are declared as empty inline modules directly in
`lib.rs` (not separate files) until their implementation blocks (`CC.1.A`/`CC.1.B`) land.

## Core Types

- **`Error`** (`src/error.rs`) — crate-level error enum via `thiserror::Error`, covering
  `BinaryNotFound`, `Spawn(std::io::Error)`, `Timeout`, and `Parse(serde_json::Error)`. Re-exported
  from `lib.rs`.
- **`Result<T>`** (`src/error.rs`) — crate-wide alias `std::result::Result<T, Error>`, re-exported
  from `lib.rs`.

Placeholder — `Config`, `Outcome`, and `Usage` are documented here as they land in `CC.1.A`/`CC.1.B`.

## Data Flow

Placeholder — caller builds a `Config` → `execute()` spawns `claude -p` (inherited or isolated env)
→ CLI emits `--output-format json` → parser extracts `total_cost_usd` + top-level `usage` + `model`
→ `Outcome` returned to the caller.
