# Changelog

All notable changes to this crate are documented here.

## Unreleased

### Fixed

- **Isolated calls no longer block a tokio worker thread.** `execute()` with `Config { isolated:
  true }` built its `IsolatedConfigDir` synchronously, so the macOS Keychain lookup
  (`security find-generic-password`) ran on the async task's own worker thread for its full
  duration. Because `securityd` serializes concurrent keychain reads, two concurrent isolated
  calls could starve the runtime — observed in `engine-rs` on 2026-09-02, where one call returned
  `Error::Timeout` against a 120s budget without ever spawning its subprocess. The construction now
  runs on tokio's blocking pool.

### Added

- `IsolatedConfigDir::new_async()` — async wrapper over the (blocking) `new()`, via
  `tokio::task::spawn_blocking`. `execute()` uses it. `new()` is unchanged and still public; its
  docs now say plainly that it blocks.

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
