---
type: Reference
title: claude-code-rs Public API
description: The public library surface — execute(), Config, Outcome — for consumers like engine-rs.
doc_id: api
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [api, library, execute, config, outcome, consumer-contract]
related: [architecture, claude-code-rs]
---

# claude-code-rs — Public API

> This crate is a **library** (no binary/CLI of its own). The stack-standard `cli.md` slot is
> replaced by this API reference. One placeholder line per section — `/document` fills these in as
> blocks ship.

## Synopsis

Placeholder — the minimal usage example (`execute(&config, prompt).await?`) goes here once `CC.1.A`
lands.

## Public Functions

Placeholder — `async fn execute(config: &Config, prompt: &str) -> Result<Outcome>` is the single
entry point; documented here with its contract when implemented.

## Config

Placeholder — builder fields mapping to CLI flags (`-p`, `--system-prompt`, `--model`,
`--allowedTools`/`--disallowedTools`, `--append-system-prompt`, `--continue`/`--resume`,
`--output-format json`) plus the env/cwd override seam for isolation.

## Outcome

Placeholder — result fields: response text, `total_cost_usd`, `usage` (`input_tokens`/
`output_tokens`/`cache_*`), and `model`.

## Consumer Contract

Placeholder — `engine-rs` `ClaudeCodeStep::process` (EN.2.A) calls `execute`, maps the result into
`TaskContext.output`, and stamps `NodeRun.usage` + cost from `total_cost_usd`. Recorded in engine-rs
decision D4.
