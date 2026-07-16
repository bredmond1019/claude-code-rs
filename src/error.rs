//! Crate-level error surface for `claude-code-rs`.

/// Errors that can occur while building, spawning, or parsing output from the
/// `claude` CLI subprocess.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `claude` binary could not be located (via `CLAUDE_BINARY` or `PATH`).
    #[error("claude binary not found")]
    BinaryNotFound,

    /// Spawning or communicating with the child process failed.
    #[error("failed to spawn claude process: {0}")]
    Spawn(#[from] std::io::Error),

    /// The call exceeded its configured timeout.
    #[error("claude call timed out")]
    Timeout,

    /// The CLI's JSON output could not be parsed into the expected shape.
    #[error("failed to parse claude output: {0}")]
    Parse(#[from] serde_json::Error),

    /// Setting up an isolated `CLAUDE_CONFIG_DIR` (temp dir creation, or a
    /// credentials/`.claude.json` source that exists but could not be read
    /// or copied) failed.
    #[error("failed to set up isolated config dir: {0}")]
    Isolation(std::io::Error),
}

/// Crate-wide `Result` alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
