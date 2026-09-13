//! Best-effort recovery for an isolated call whose frozen credential snapshot has gone stale.
//!
//! [`crate::isolation::IsolatedConfigDir`] deliberately strips `refreshToken` from the copy it
//! hands an isolated subprocess, so that subprocess can never consume the single-use refresh
//! token and revoke a concurrent interactive session's credentials. The consequence: an isolated
//! call can never self-heal an expired or invalid access token on its own — it only ever gets a
//! frozen snapshot at copy time. This module identifies that exact failure shape
//! ([`is_isolated_auth_expired`]) and, when the caller opts in, provides the recovery mechanism
//! ([`heal_shared_credentials`], wired into [`crate::execute::execute`]).

use crate::error::Error;

/// Returns `true` only when `err` is the exact failure shape of an isolated call whose credential
/// snapshot has gone stale: an [`Error::Api`] with HTTP status `401` whose message contains the
/// substring `"OAuth"`.
///
/// This is deliberately narrow. Every other [`Error`] variant, and every other `Error::Api` shape
/// (a non-401 status, or a 401 with an unrelated message), returns `false` — the predicate must
/// gate the whole heal-and-retry mechanism to this one recoverable case, not to every isolated
/// failure.
// Not yet called from `execute()` — that wiring lands in a later task of this ticket. Allowed
// here (rather than left to trip `-D warnings`) because this module's own tests already exercise
// it against the real captured fixture; removing this attribute is part of that later task.
#[allow(dead_code)]
#[must_use]
pub(crate) fn is_isolated_auth_expired(err: &Error) -> bool {
    matches!(
        err,
        Error::Api {
            status: Some(401),
            message,
            ..
        } if message.contains("OAuth")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const FIXTURE: &str = include_str!("../tests/fixtures/cli-error-oauth-expired-2.1.270.json");

    /// Builds the exact `Error::Api` shape `execute()` itself produces from an `is_error`
    /// envelope, so this test exercises the real conversion path rather than a hand-built value.
    fn api_error_from_envelope(json: &str) -> Error {
        let outcome = parse::parse_result(json).expect("fixture must parse");
        assert!(outcome.is_error, "fixture must be an error envelope");
        Error::Api {
            status: outcome.api_error_status,
            message: outcome.text,
            session_id: outcome.session_id,
            cost_usd: outcome.cost_usd,
            usage: outcome.usage,
        }
    }

    #[test]
    fn true_for_the_real_captured_fixture() {
        let err = api_error_from_envelope(FIXTURE);
        assert!(
            is_isolated_auth_expired(&err),
            "the real captured 401/OAuth envelope must match, got {err:?}"
        );
    }

    #[test]
    fn true_for_the_second_real_production_message_text() {
        let err = Error::Api {
            status: Some(401),
            message: "Failed to authenticate. API Error: 401 OAuth access token has expired. Re-authenticate to continue.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: parse::Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        assert!(is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_timeout() {
        assert!(!is_isolated_auth_expired(&Error::Timeout));
    }

    #[test]
    fn false_for_binary_not_found() {
        assert!(!is_isolated_auth_expired(&Error::BinaryNotFound));
    }

    #[test]
    fn false_for_cli_error() {
        let err = Error::Cli {
            status: Some(1),
            stderr: "anything at all, including the word OAuth".to_string(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_non_401_status_with_oauth_message() {
        let err = Error::Api {
            status: Some(500),
            message: "OAuth access token is invalid.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: zero_usage(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    #[test]
    fn false_for_401_status_without_oauth_in_message() {
        let err = Error::Api {
            status: Some(401),
            message: "Unauthorized.".to_string(),
            session_id: None,
            cost_usd: 0.0,
            usage: zero_usage(),
        };
        assert!(!is_isolated_auth_expired(&err));
    }

    fn zero_usage() -> parse::Usage {
        parse::Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }
    }
}
