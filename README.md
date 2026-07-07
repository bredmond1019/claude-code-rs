---
type: Index
title: claude-code-rs
description: A lean async Rust SDK that runs Claude Code as a subprocess on Brandon's flat-rate subscription (not API credits).
doc_id: readme
layer: [factory]
status: active
keywords: [project readme, prerequisites, setup, getting started]
related: [context, master-plan, planning-index]
---

# claude-code-rs

> Part of the **Bastion** ecosystem — see the [bastion-os](https://github.com/bredmond1019/bastion-os) front door for the full architecture.

A lean async Rust SDK that runs Claude Code as a subprocess on Brandon's flat-rate subscription (not API credits).

## Prerequisites

- Rust 1.78+ (via rustup)

## Setup

```bash
# 1. Clone the repository
git clone https://github.com/bredmond1019/claude-code-rs
# 2. Build the project
cargo build
```

## Running locally

```bash
cargo run --release
```

## Tests

```bash
cargo test
```

## Directory map

```
claude-code-rs/
├── .claude/        ← Claude Code commands + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/, <concept>/
└── <source dirs>
```

## Documentation

| Doc | Contents |
|---|---|
| `planning/context.md` | Orientation + governing principles |
| `planning/master-plan.md` | Strategy + phase specifications |
| `planning/status.md` | Current progress |
| `planning/harness.json` | SDLC validation/UI-test config (see `harness.examples.md`) |

## Roadmap / Known limitations

- **Zero-Allocation Deserialization:** The crate currently buffers content blocks through an intermediate `serde_json::Value`. Implementing a zero-alloc `serde::de::Visitor` is a planned refinement to avoid the round-trip.

---

*Initialized 2026-07-03 from `base-template` (commit `9ea6decce523300fb82ad18a65f50272edab7702`).*
