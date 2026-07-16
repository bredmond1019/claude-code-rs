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

    /// The `claude` CLI itself failed before producing a response envelope —
    /// bad arguments, a missing prompt, a crash. Diagnosed by an empty stdout;
    /// the message is on stderr.
    ///
    /// Distinct from [`Error::Api`]: here the CLI never reached the API.
    #[error("claude CLI failed (exit {status:?}): {stderr}")]
    Cli {
        /// Process exit code, if the process was not killed by a signal.
        status: Option<i32>,
        /// The CLI's stderr, trimmed.
        stderr: String,
    },

    /// The CLI ran and emitted a well-formed envelope reporting a failure
    /// (`is_error: true`) — e.g. an unroutable model, or an API outage.
    ///
    /// Distinct from [`Error::Cli`]: the CLI worked; the API call did not.
    /// Note the message arrives on *stdout*, inside the JSON's `result` field —
    /// stderr is empty on this path.
    #[error("claude API error{}: {message}", .status.map(|s| format!(" (HTTP {s})")).unwrap_or_default())]
    Api {
        /// HTTP status from the envelope's `api_error_status`, when reported.
        status: Option<u16>,
        /// Human-readable message from the envelope's `result` field.
        message: String,
    },

    /// Setting up an isolated `CLAUDE_CONFIG_DIR` (temp dir creation, or a
    /// credentials/`.claude.json` source that exists but could not be read
    /// or copied) failed.
    #[error("failed to set up isolated config dir: {0}")]
    Isolation(std::io::Error),
}

/// Crate-wide `Result` alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
