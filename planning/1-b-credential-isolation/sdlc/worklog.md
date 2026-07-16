# Worklog — 1-b-credential-isolation

## Task 1 — PASSED (1 attempt)
What: Added the isolation module: IsolatedConfigDir RAII guard builds a temp CLAUDE_CONFIG_DIR with a refreshToken-redacted .credentials.json (mode 0600) and an optional .claude.json copy, sourced from macOS Keychain then ~/.claude/.credentials.json fallback in production or an injectable with_sources() constructor in tests; Drop removes the temp dir, including on mid-construction failure.
Decisions: Used tempfile::TempDir as the backing store for IsolatedConfigDir instead of hand-rolled temp-dir management — its own Drop already guarantees cleanup on every exit path (including mid-construction failure), so no manual cleanup-on-error logic was needed.; Modeled the 'injectable credential source' requirement as two named constructors (new() for the real Keychain/file sources, with_sources(creds_json, claude_json_src) for tests/DI) rather than a trait or closure parameter — simpler API surface for the one call site.; Added Error::Isolation(std::io::Error) without #[from] (unlike Error::Spawn) because thiserror can't derive two From<std::io::Error> impls on the same enum; call sites use .map_err(Error::Isolation) explicitly.; copy_if_present only suppresses io::ErrorKind::NotFound (mirroring the Python reference's suppress(FileNotFoundError)) — any other I/O error (e.g. src is a directory) propagates as a build failure, which is what the mid-construction-failure test exercises.; Added a private build(parent, ...) seam (test-only 'parent' dir param) so unit tests can assert the temp dir's parent is empty after a failed build, without adding a public API for it.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Config now carries an opt-in isolated: bool field (default false) alongside the now-documented-as-consumed cwd/env override fields, with build_args() and its tests untouched.
Decisions: Placed isolated field after cwd/env to match the order tasks.md describes them; doc comments reference the isolation module by name since task 1 already landed it.
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: execute() now applies config.cwd via Command::current_dir, config.env via Command::envs, and — when config.isolated is true — builds an IsolatedConfigDir guard, sets CLAUDE_CONFIG_DIR in the child env, and keeps the guard alive until after the child's output is read; the default (non-isolated, no overrides) path is unchanged.
Decisions: Built the isolation guard before the async spawn block so a mid-setup Error::Isolation surfaces before ever spawning the child, and explicitly `drop(isolation_guard)` after the timeout race resolves (rather than relying on end-of-scope) to make the 'guard outlives the child' contract visible in the code.; Added 4 new unix-only unit tests in src/execute.rs (not tests/isolation.rs, which is task 4's scope) that swap CLAUDE_BINARY for a tiny generated shell script reporting $PWD/env vars back through the Outcome.model field, to observe what execute() actually applied to the child Command without invoking the real claude CLI.; Added a static Mutex to serialize all CLAUDE_BINARY env-var mutations across tests in execute.rs's test module, since cargo test runs tests in parallel by default and the existing resolve_binary test already mutated this same process-global var.
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Added tests/isolation.rs integration tests covering isolated temp-dir layout (redacted .credentials.json mode 0600 + copied .claude.json), Drop cleanup, and an #[ignore]d live concurrent execute() smoke test.
Decisions: Reused the existing public API (IsolatedConfigDir::with_sources, execute() with Config{isolated:true}) rather than adding new test-only hooks, since task 1/2/3 already expose everything task 4 needs.; Live test runs the isolated and a normal execute() concurrently via tokio::join! to directly exercise the 'does not disturb an interactive session' acceptance criterion rather than just asserting a populated Outcome in isolation.
Validated: gating checks (fast tripwire)

## Task 5 — PASSED (1 attempt)
What: Task 5 (validation-only) confirmed all four gated checks pass: cargo fmt --check, cargo clippy -- -D warnings, cargo test (23 passed, 2 ignored live tests), cargo build --release — no code changes required.
Decisions: No commit made: task 5 is a pure validation gate with no files listed in tasks.json and the working tree was already clean after tasks 1-4.
Validated: gating checks (fast tripwire)

## Docs
Patched: docs/architecture.md, docs/api.md

## Wrap-up — PASS
Next: Phase 1, Block C or Phase 2, Block A (CC.2.A — Streaming output); define tasks via /generate-tasks

## PR
https://github.com/bredmond1019/claude-code-rs/pull/2
