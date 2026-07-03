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
├── lib.rs        ← crate root; re-exports the public surface
├── config.rs     ← Config → CLI arg-builder (stub)
├── execute.rs    ← async execute(): spawn, timeout, kill_on_drop (stub)
├── parse.rs      ← CLI stream-json / result parsing, forward-compat deserialize (stub)
├── isolation.rs  ← CLAUDE_CONFIG_DIR temp dir + redacted credentials (stub)
└── error.rs      ← thiserror error type (stub)
```

## Core Types

Placeholder — `Config`, `Outcome`, `Usage`, and the crate `Error` are documented here as they land.

## Data Flow

Placeholder — caller builds a `Config` → `execute()` spawns `claude -p` (inherited or isolated env)
→ CLI emits `--output-format json` → parser extracts `total_cost_usd` + top-level `usage` + `model`
→ `Outcome` returned to the caller.
