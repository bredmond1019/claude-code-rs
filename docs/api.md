---
type: Reference
title: claude-code-rs Public API
description: The public library surface — execute(), Config, Outcome — for consumers like engine-rs.
doc_id: api
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [api, library, execute, config, outcome, consumer-contract]
related: [architecture]
---

# claude-code-rs — Public API

> This crate is a **library** (no binary/CLI of its own). The stack-standard `cli.md` slot is
> replaced by this API reference. One placeholder line per section — `/document` fills these in as
> blocks ship.

## Synopsis

```rust
use claude_code_rs::{Config, execute};

let config = Config::default();
let outcome = execute(&config, "Say hello in one word.").await?;
println!("{} cost ${}", outcome.model, outcome.cost_usd);
```

## Public Functions

- **`async fn execute(config: &Config, prompt: &str) -> Result<Outcome>`** (`src/execute.rs`) — the
  single entry point. Resolves the `claude` binary (`CLAUDE_BINARY` env var, else `PATH` lookup via
  `which`), spawns it with `config.build_args(prompt)` (env inherited from the current process),
  captures stdout, and wraps the whole call in one `tokio::time::timeout` (default 300s; not
  per-line), killing the child on drop/timeout. Errors: `Error::BinaryNotFound`, `Error::Spawn`,
  `Error::Timeout`, `Error::Parse`.

## Config

`Config` (`src/config.rs`, `Debug + Clone + Default`) fields mapping to CLI flags:

| Field | CLI flag |
|---|---|
| `system_prompt: Option<String>` | `--system-prompt` |
| `append_system_prompt: Option<String>` | `--append-system-prompt` |
| `model: Option<String>` | `--model` |
| `allowed_tools: Vec<String>` | `--allowedTools` (repeated) |
| `disallowed_tools: Vec<String>` | `--disallowedTools` (repeated) |
| `continue_session: bool` | `--continue` |
| `resume: Option<String>` | `--resume <id>` |
| `cwd: Option<PathBuf>` | placeholder — not yet consumed by `execute()`, reserved for `CC.1.B` |
| `env: Vec<(String, String)>` | placeholder — not yet consumed by `execute()`, reserved for `CC.1.B` |

`Config::build_args(&self, prompt: &str) -> Vec<String>` builds the exact argv: `-p <prompt>`, then
the flags above in field order, always ending with `--output-format json`.

## Outcome

`Outcome` (`src/parse.rs`, schema-locked to today's `claude` CLI JSON — `total_cost_usd`, top-level
`usage`, `model`, not the stale `cost_usd`/`message.usage` shape from earlier CLI versions):

- `cost_usd: f64` — from `total_cost_usd`.
- `usage: Usage` — `input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens` (each defaulted to `0` if absent).
- `model: String`.
- `content: Vec<ContentBlock>` — defaults to empty; `ContentBlock::Text { text }` or
  `ContentBlock::Unknown { block_type, data }` for forward-compat with unrecognized block types.

`parse::parse_result(json: &str) -> Result<Outcome>` parses a raw `claude` CLI JSON response;
returns `Error::Parse` if invalid or missing a required field (`total_cost_usd`, `usage`, `model`).

## Consumer Contract

Placeholder — `engine-rs` `ClaudeCodeStep::process` (EN.2.A) calls `execute`, maps the result into
`TaskContext.output`, and stamps `NodeRun.usage` + cost from `total_cost_usd`. Recorded in engine-rs
decision D4.
