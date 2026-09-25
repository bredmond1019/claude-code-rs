//! Crate-level error surface for `claude-code-rs`.

use crate::parse::Outcome;

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
        /// The CLI session this failed call ran under, from the envelope's
        /// `session_id` (see [`crate::parse::Outcome::session_id`]).
        ///
        /// Carried on the error path deliberately: this is the one failure mode where a real,
        /// token-consuming session exists — the CLI ran, reached the API, and billed for the
        /// attempt. `Spawn`/`BinaryNotFound`/`Timeout`/`Parse` have no envelope and so no id at
        /// all. Dropping it would understate exactly the runs a cost comparison cares about most.
        ///
        /// `None` if the envelope carried no `session_id`.
        session_id: Option<String>,
        /// The envelope's `total_cost_usd`. Carried for the same reason as `session_id`: the CLI
        /// reached the API and billed for this attempt, and the error envelope reports the charge
        /// like any other. Often `0` (a model that does not exist never ran), but not always — an
        /// overload or timeout after real work bills for that work.
        cost_usd: f64,
        /// The envelope's `usage` block. Same reasoning as `cost_usd`.
        usage: crate::parse::Usage,
    },

    /// The CLI ran, reached the API, and stopped only because it hit its
    /// configured turn ceiling (`is_error: true`, `subtype: error_max_turns`).
    ///
    /// Distinct from [`Error::Api`]: this is not a malformed response or an
    /// API-side failure — the call simply ran out of turns. `Error::Api`
    /// covers every OTHER `is_error` shape.
    ///
    /// The whole envelope is boxed and kept intact — never destructured
    /// field-by-field the way [`Error::Api`] is — so the caller keeps the
    /// billed envelope's `cost_usd`, `usage`, `session_id`, `num_turns`, and
    /// `errors` together for attribution.
    #[error("claude call hit its turn ceiling ({:?} turns, subtype {:?})", .0.num_turns, .0.subtype)]
    MaxTurns(Box<Outcome>),

    /// Setting up an isolated `CLAUDE_CONFIG_DIR` (temp dir creation, or a
    /// credentials/`.claude.json` source that exists but could not be read
    /// or copied) failed.
    #[error("failed to set up isolated config dir: {0}")]
    Isolation(std::io::Error),

    /// `Config::dangerously_skip_permissions` and `Config::permission_mode`
    /// are alternative ways of controlling permissions; both are set at
    /// once. Returned by [`crate::config::Config::validate`], which
    /// [`crate::execute::execute`] calls before resolving the binary.
    #[error(
        "dangerously_skip_permissions and permission_mode are mutually exclusive — set at most one"
    )]
    ConflictingPermissions,
}

/// Crate-wide `Result` alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
