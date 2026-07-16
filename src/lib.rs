//! Lean async Rust SDK that runs the `claude` CLI as a subprocess on a flat-rate subscription.

mod error;

pub mod config;
pub mod execute;
pub mod isolation;
pub mod parse;

pub use config::Config;
pub use error::{Error, Result};
pub use execute::execute;
pub use isolation::IsolatedConfigDir;
pub use parse::Outcome;

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
