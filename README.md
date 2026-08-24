---
type: Index
title: claude-sdk-rs
description: A lean async Rust SDK that runs Claude Code as a subprocess on a flat-rate subscription (not metered API credits).
doc_id: readme
layer: [factory]
status: active
keywords: [project readme, prerequisites, setup, getting started]
related: [context, master-plan, planning-index]
---

# claude-sdk-rs

[![Crates.io](https://img.shields.io/crates/v/claude-sdk-rs.svg)](https://crates.io/crates/claude-sdk-rs)
[![docs.rs](https://img.shields.io/docsrs/claude-sdk-rs)](https://docs.rs/claude-sdk-rs)

A lean async Rust SDK that runs the [Claude Code](https://claude.ai/code) CLI as a subprocess,
authenticated through a flat-rate Claude subscription rather than metered API credits. If you
already pay for Claude Code, this crate lets you call it programmatically from Rust without a
separate API key or per-token billing.

**2.0.0 is a ground-up rewrite of the `claude-sdk-rs` 1.x line, with no migration path.** See
[CHANGELOG.md](./CHANGELOG.md) for what changed and why. If you depend on 1.x, pin
`claude-sdk-rs = "=1.0.2"` — that version is frozen on the [`legacy-v1`](https://github.com/bredmond1019/claude-sdk-rs/tree/legacy-v1)
branch.

## How it works

There is no HTTP client here. `execute()` spawns the `claude` binary as a subprocess
(`tokio::process::Command`, `.kill_on_drop(true)`, wrapped in a single whole-call timeout), passes
your prompt and options as CLI flags, and parses the CLI's `--output-format json` response into a
typed `Outcome`. Authentication is whatever the `claude` CLI itself is already using — your Claude
subscription login, sourced from the macOS Keychain or `~/.claude/.credentials.json`.

**Platform note:** the built-in credential isolation (`Config.isolated`) sources credentials via
the macOS Keychain with a file fallback. It has only been exercised on macOS; other platforms may
need the file-based fallback path exclusively.

## Prerequisites

- Rust 1.78+ (via rustup)
- The [`claude` CLI](https://claude.ai/code) installed and logged in (`claude auth login`), on
  `PATH` or pointed to via the `CLAUDE_BINARY` env var
- A Claude subscription (Pro/Max) — this crate does not use API keys

## Install

```bash
cargo add claude-sdk-rs
```

## Example

```rust,no_run
use claude_sdk_rs::{execute, Config};

#[tokio::main]
async fn main() -> claude_sdk_rs::Result<()> {
    let config = Config::default();
    let outcome = execute(&config, "What is 2 + 2?").await?;
    println!("{}", outcome.text);
    Ok(())
}
```

`Config` also covers `system_prompt`, `model`, `allowed_tools`/`disallowed_tools`,
`continue_session`/`resume` (multi-turn), `json_schema` (structured output), `isolated`
(credential isolation for concurrent callers), and `timeout` (per-call override of the 300s
default). See [docs.rs](https://docs.rs/claude-sdk-rs) for the full API.

## Why a subprocess wrapper instead of an HTTP client

Two differentiators this crate is built around:

- **Subscription auth, not metered API keys.** Every call rides your existing Claude Code login —
  no separate API billing to configure or reconcile.
- **Schema locked to captured CLI output, not memory.** `tests/fixtures/` holds real captured CLI
  responses; `tests/parse_schema.rs` asserts against them, and an ignored canary test diffs live CLI
  output against the fixtures on demand — so a CLI schema change surfaces as a failing test, not
  silent drift in production.

## Tests

```bash
cargo test
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) · <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) · <http://opensource.org/licenses/MIT>)

at your option. Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.

Built for one operator and released because it may be useful to others — there is no support
obligation, no issue-response SLA, and no stability promise.
