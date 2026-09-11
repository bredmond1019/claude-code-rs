---
type: Reference
title: claude-code-rs Public API
description: The public library surface — execute(), Config, Outcome — for consumers like engine-rs.
doc_id: api
layer: [engine, infra]
project: claude-code-rs
status: active
keywords: [api, library, execute, config, outcome, consumer-contract, isolation]
related: [architecture]
---

# claude-code-rs — Public API

The complete public surface, field by field. This crate is a **library** — there is no binary or
CLI of its own, so this page takes the place of the usual `cli.md`. If you want to get something
running rather than look a field up, start at the repo [`README.md`](../README.md); if you want to
know how a call flows, read [architecture.md](architecture.md).

## Quickstart

The crate is published as **`claude-sdk-rs`** — the repo directory is `claude-code-rs`, but the
crate name and the Rust module path are `claude_sdk_rs`.

```bash
cargo add claude-sdk-rs
```

One call, default everything:

```rust
use claude_sdk_rs::{execute, Config};

let config = Config::default();
let outcome = execute(&config, "Say hello in one word.").await?;
println!("{} cost ${}", outcome.text, outcome.cost_usd);
// The model name is NOT a top-level field — see `Outcome::primary_model()` below.
println!("served by {:?}", outcome.primary_model());
```

## Public Functions

One function does the work. Everything else on this page either configures it or describes what it
gives back.

- **`async fn execute(config: &Config, prompt: &str) -> Result<Outcome>`** (`src/execute.rs`) — the
  single entry point. Resolves the `claude` binary (`CLAUDE_BINARY` env var, else `PATH` lookup via
  `which`), applies `config.cwd` and `config.env` to the child `Command`, and — when
  `config.isolated` is `true` — builds an `IsolatedConfigDir` guard and sets `CLAUDE_CONFIG_DIR` in
  the child env (guard is kept alive until the child's output is read, then dropped, cleaning up the
  temp dir). Spawns the process with `config.build_args(prompt)` (env otherwise inherited from the
  current process), captures stdout, and wraps the whole call in one `tokio::time::timeout` (not
  per-line) whose duration is `config.timeout` when set, else the built-in `DEFAULT_TIMEOUT` of
  300s, killing the child on drop/timeout. Errors: `Error::BinaryNotFound`,
  `Error::Spawn`, `Error::Timeout`, `Error::Parse`, `Error::Isolation`.

## Config

`Config` describes a single call. It derives `Default`, so most callers write `Config::default()`
and override one or two fields with struct-update syntax. Most fields become a CLI flag on the
spawned `claude` process; four (`cwd`, `env`, `isolated`, `timeout`) are handled Rust-side and
never appear in argv.

`Config` (`src/config.rs`, `Debug + Clone + Default`) fields:

| Field | CLI flag |
|---|---|
| `system_prompt: Option<String>` | `--system-prompt` |
| `append_system_prompt: Option<String>` | `--append-system-prompt` |
| `model: Option<String>` | `--model` |
| `allowed_tools: Vec<String>` | `--allowedTools` (repeated) |
| `disallowed_tools: Vec<String>` | `--disallowedTools` (repeated) |
| `continue_session: bool` | `--continue` |
| `resume: Option<String>` | `--resume <id>` |
| `cwd: Option<PathBuf>` | applied via `Command::current_dir` (not a CLI flag) |
| `env: Vec<(String, String)>` | applied via `Command::envs`, on top of the inherited environment (not a CLI flag) |
| `isolated: bool` | when `true`, `execute()` runs the subprocess under a temp `CLAUDE_CONFIG_DIR` built by `IsolatedConfigDir` (see below); not a CLI flag; default `false` |
| `dangerously_skip_permissions: bool` | `--dangerously-skip-permissions` — appended only when `true`; omitted entirely at the `false` default, so existing callers are unaffected. Grants no tool a wider reach than the CLI's own tool definitions allow; scoping the blast radius (e.g. via `cwd` plus `disallowed_tools`) is the caller's responsibility |
| `json_schema: Option<serde_json::Value>` | `--json-schema <json>` — when `Some`, serialized to compact JSON and emitted immediately before the trailing `--output-format json` pair; omitted entirely when `None` (default) |
| `max_turns: Option<u32>` | `--max-turns <n>` — emitted only when `Some`; omitted entirely when `None` (default) |
| `timeout: Option<Duration>` | overrides `execute()`'s whole-call `tokio::time::timeout`; not a CLI flag (never appears in `build_args`). `None` (default) keeps the built-in `DEFAULT_TIMEOUT` of 300s, so existing callers are unaffected; `Some(duration)` widens or narrows it for that call |

`Config::build_args(&self, prompt: &str) -> Vec<String>` builds the exact argv: `-p <prompt>`, then
the flags above in field order, always ending with `--output-format json`. `cwd`, `env`,
`isolated`, and `timeout` are not CLI flags — the first three are applied to the `Command` directly
by `execute()`, and `timeout` only sets the duration of `execute()`'s Rust-side timeout, so
`build_args`'s output is byte-identical whatever it is set to.

## IsolatedConfigDir

Interactive `claude` sessions and SDK-driven subprocess calls share one credentials file. If a
background call refreshes the OAuth token, the single-use refresh token is consumed and your
interactive session is silently logged out. This guard is the fix.

`IsolatedConfigDir` (`src/isolation.rs`) — an RAII guard used when `Config::isolated` is `true`. It
builds a throwaway directory laid out like `~/.claude/`, suitable for pointing an isolated
subprocess at via `CLAUDE_CONFIG_DIR`, so a concurrent SDK-driven call cannot consume the
single-use OAuth refresh token and silently revoke an interactive session's credentials.

- **`IsolatedConfigDir::new() -> Result<Self>`** — real credential sources: the macOS Keychain
  first (best-effort, falls through on any failure), then `~/.claude/.credentials.json`;
  `.claude.json` is copied from `~/.claude.json` when present. Errors: `Error::Isolation`.
  **Blocking** — the Keychain lookup shells out to `security` and waits for it. Do not call this
  from an async task; use `new_async()`.
- **`IsolatedConfigDir::new_async() -> Result<Self>`** (async) — what `execute()` calls. Identical
  behavior to `new()`, run on tokio's blocking pool via `tokio::task::spawn_blocking`. macOS's
  `securityd` serializes concurrent keychain reads, so on `new()` that wait parks a tokio *worker*
  thread and stalls every other task on the runtime — a concurrent `execute()` has been observed
  returning `Error::Timeout` from this starvation alone, without ever spawning its subprocess.
  Additionally errors with `Error::Isolation` wrapping a `JoinError` if the blocking task panicked.
- **`IsolatedConfigDir::with_sources(creds_json: Option<String>, claude_json_src: Option<&Path>) -> Result<Self>`**
  — injectable constructor for tests/DI, bypassing the Keychain and `~/.claude/` entirely.
- **`IsolatedConfigDir::path(&self) -> &Path`** — absolute path to the temp `CLAUDE_CONFIG_DIR`.
- The written `.credentials.json` has its `refreshToken` field deleted (mode `0600`) so the
  subprocess's refresh check short-circuits. Dropping the guard removes the whole temp directory,
  including on a mid-construction failure.

## Outcome

What you get back on success: the reply text, what it cost, how many tokens it used, and which
model served it.

`Outcome` (`src/parse.rs`) mirrors the CLI's `--output-format json` envelope. **The authority for
this shape is `tests/fixtures/` — real captured responses — not this page.** If the two disagree,
the fixtures are right and this page is stale. See [`tests/fixtures/README.md`](../tests/fixtures/README.md) and this repo's decision D2
(`planning/decisions/D2-cli-schema-provenance.md` — in the gitignored brain vault, not on GitHub).

- `cost_usd: f64` — from `total_cost_usd`.
- `usage: Usage` — `input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
  `cache_read_input_tokens` (each defaulted to `0` if absent). The CLI's `usage` object carries
  further fields that this crate ignores.
- `model_usage: BTreeMap<String, ModelUsage>` — from `modelUsage`. **The only place the model name
  appears**; there is no top-level `model` field. Empty on the error envelope.
- `text: String` — from `result`. The reply on success, the error message on failure. Required: a
  default would render its removal as an empty reply, which is silent data loss.
- `is_error: bool` — the only trustworthy failure signal. The envelope reports
  `subtype: "success"` even when the call failed, so `subtype` must never be used for this.
- `api_error_status: Option<u16>` — `None` on success.
- `structured_output: Option<serde_json::Value>` — from `structured_output`, present only when the
  call was made with `Config.json_schema` set (CLI: `--json-schema`); absent (not `null`) on a
  schemaless call. `text` still carries the same JSON as a string via `result` either way — this
  field is the pre-parsed object form.

`Outcome::primary_model() -> Option<&str>` picks the most plausible model from `model_usage`,
ranking by cost, then output tokens, then key order. This is **this crate's heuristic**, not
something the CLI reports — a single call can bill several models. `None` when no model ran.

`parse::parse_result(json: &str) -> Result<Outcome>` parses a raw `claude` CLI JSON response;
returns `Error::Parse` if invalid or missing a required field (`total_cost_usd`, `usage`, `result`,
`is_error`). Unknown fields are ignored, so a vendor addition never breaks the parse.

## Consumer Contract

Who else depends on this surface, and why that dependency needs no version pin.

`engine-rs`'s `ClaudeCodeStep::process` (EN.2.A) calls `execute`, writes
`{content, cost_usd, model}` into its own `TaskContext::nodes` entry, and stamps `NodeRun.usage`.
Recorded in engine-rs decision D4.

Note this boundary needs no version pin or contract doc: `engine-rs` consumes this crate through a
Cargo **path dependency** and constructs `Outcome` as a struct literal, so `rustc` enforces the
seam on every build — more strictly than prose could. The boundary that *does* need evidence is the
one above it, between this crate and the vendor's CLI, which is what the fixtures cover.
