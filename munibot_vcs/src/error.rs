//! The error type every forge-agnostic trait in this crate returns.

use thiserror::Error;

/// Why a call against [`crate::IssueSource`] or [`crate::PullRequestTarget`]
/// failed.
///
/// Deliberately forge-agnostic: a specific implementation (`munibot_github`'s
/// `GitHubForge`) translates whatever its own client library raises into one
/// of these variants, rather than the pipeline ever matching on an
/// octocrab error type.
#[derive(Error, Debug)]
pub enum VcsError {
    /// The referenced repository, issue, or pull request does not exist, or
    /// the credentials in use cannot see it.
    #[error("{0} wasn't found, or the installation can't see it :<")]
    NotFound(String),
    /// The credentials in use are valid but lack permission for this call.
    #[error("not allowed to do that: {0}")]
    Forbidden(String),
    /// The forge is rate-limiting this installation.
    #[error("rate limited by the forge, try again later")]
    RateLimited,
    /// Authentication itself failed -- an expired token, a malformed key,
    /// a revoked installation.
    #[error("couldn't authenticate with the forge: {0}")]
    Authentication(String),
    /// Anything else, carrying the underlying client's own message.
    #[error("forge call failed: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_names_what_was_missing() {
        let error = VcsError::NotFound("musicaloft/munibot#42".to_string());
        assert!(error.to_string().contains("musicaloft/munibot#42"));
    }

    #[test]
    fn test_forbidden_names_the_reason() {
        let error = VcsError::Forbidden("no write access".to_string());
        assert!(error.to_string().contains("no write access"));
    }

    #[test]
    fn test_rate_limited_has_a_stable_message() {
        assert_eq!(
            VcsError::RateLimited.to_string(),
            "rate limited by the forge, try again later"
        );
    }
}
