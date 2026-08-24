# Changelog

All notable changes to this crate are documented here.

## 2.0.0 — 2026-08-24

**This is a ground-up rewrite, not an incremental release. There is no migration path from 1.x.**

`claude-sdk-rs` 1.x (published as this crate's original artifact) wrapped the Claude Code CLI with
a stale response schema (`cost_usd` / `message.usage`), no `kill_on_drop` on the spawned subprocess,
a dead `SessionManager`, two divergent stream-JSON parsers, a hardcoded 30s timeout, and no
`CLAUDE_CONFIG_DIR` credential-isolation seam. 2.0.0 replaces all of it with a lean, from-scratch
implementation:

- Async subprocess transport (`execute()`) over `tokio::process::Command` with `.kill_on_drop(true)`
  and a single whole-call timeout (default 300s, configurable via `Config.timeout`).
- Response parsing locked to the CLI's real `--output-format json` schema via captured fixtures
  (`tests/fixtures/`), with a drift canary that fails on both missing *and* added vendor fields.
- Opt-in credential isolation (`Config.isolated`): runs the subprocess under a temp
  `CLAUDE_CONFIG_DIR` with a redacted credentials copy, so a concurrent subprocess session cannot
  log out an interactive one.
- Structured output support (`Config.json_schema` / `Outcome.structured_output`).
- A minimal, audited dependency set: `tokio`, `serde`, `serde_json`, `thiserror`, `which`.

**If you depend on 1.x, pin `claude-sdk-rs = "=1.0.2"`.** That version is frozen and will not
receive further updates; its source remains on the `legacy-v1` branch of this repository.

## 1.0.2 and earlier

Predates this rewrite. See the `legacy-v1` branch for that source history.
