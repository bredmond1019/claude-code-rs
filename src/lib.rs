//! Lean async Rust SDK that runs the `claude` CLI as a subprocess on a flat-rate subscription.

mod error;
mod heal;
mod permission_hook;
mod process_group;
mod settings;

pub mod command_hook;
pub mod config;
pub mod confine;
pub mod endpoint;
pub mod execute;
pub mod isolation;
pub mod parse;

pub use command_hook::CommandHook;
pub use config::{Config, ParsePermissionModeError, PermissionMode};
pub use endpoint::Endpoint;
pub use error::{Error, Result};
pub use execute::execute;
pub use isolation::IsolatedConfigDir;
pub use parse::Outcome;
pub use parse::PermissionDenial;
pub use parse::ResultSubtype;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_composes() {
        let ok: Result<()> = Ok(());
        assert!(ok.is_ok());

        let err: Result<()> = Err(Error::Timeout);
        assert!(err.is_err());
    }
}
