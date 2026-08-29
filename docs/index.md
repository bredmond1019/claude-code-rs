---
type: Index
title: claude-code-rs Docs
description: Navigation index and capability catalogue for claude-code-rs — every public API item, what it does, and where it is documented.
doc_id: docs-index
layer: [meta]
project: claude-code-rs
status: active
keywords: [documentation, index, capability-catalogue, api-surface, navigation]
related: [api, architecture]
---

# claude-code-rs — Documentation Index

This crate is a **library** — there is no CLI of its own. "What it can do" is therefore its public
Rust surface: one entry point, one config struct, one result type, and an opt-in credential guard.

## Quickstart

New here? Start with the repo [`README.md`](../README.md) — prerequisites, `cargo add`, and a
runnable ten-line example. Come back here when you need the field-by-field detail.

```bash
cargo add claude-sdk-rs     # published crate name; repo/directory name is claude-code-rs
cargo test                  # confirms your setup — no live `claude` call
```

## Capability catalogue

Everything the crate exports, derived from `pub use` in `src/lib.rs`. One line each; the linked
doc is the authority.

| What you can do | Public item | Where it's documented |
|---|---|---|
| Run one prompt through the `claude` CLI and get a typed result | `execute(&Config, &str)` (`src/execute.rs`) | [api.md](api.md#public-functions) |
| Describe a single call — model, tools, system prompt, cwd, timeout | `Config` (`src/config.rs`) | [api.md](api.md#config) |
| See the exact argv a `Config` produces, without running anything | `Config::build_args(prompt)` | [api.md](api.md#config) |
| Read the reply, its cost, and its token usage | `Outcome` (`src/parse.rs`) | [api.md](api.md#outcome) |
| Ask which model actually served the call | `Outcome::primary_model()` | [api.md](api.md#outcome) |
| Parse a raw CLI JSON response you captured yourself | `parse::parse_result(json)` | [api.md](api.md#outcome) |
| Stop a background call from logging out your interactive `claude` session | `Config { isolated: true }` / `IsolatedConfigDir` (`src/isolation.rs`) | [api.md](api.md#isolatedconfigdir) |
| Tell apart a CLI failure from an API failure from a timeout | `Error` / `Result` (`src/error.rs`) | [architecture.md](architecture.md#core-types) |

## Reference docs

| Doc | What it covers |
|---|---|
| [architecture.md](architecture.md) | How a call flows through the crate; module map; core types; failure routing |
| [api.md](api.md) | Field-by-field public surface and the `engine-rs` consumer contract |
| [`tests/fixtures/README.md`](../tests/fixtures/README.md) | Provenance of the captured CLI responses — the authority for `Outcome`'s shape |
| [`hooks/README.md`](../hooks/README.md) | The repo's git hooks and how to enable them |
| [`CHANGELOG.md`](../CHANGELOG.md) | What changed in 2.0.0 and why |

Project strategy and current focus live in the company-brain vault at
`agentic-portfolio/core/_planning/claude-code-rs/` (surfaced locally as `planning/`, which is
gitignored and therefore not linkable from a public page).
