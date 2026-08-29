//! The error type this crate's own operations return.

use thiserror::Error;

/// Why a github-specific operation failed, before it is ever translated
/// into a forge-agnostic [`munibot_vcs::VcsError`] at the trait boundary.
#[derive(Error, Debug)]
pub enum GitHubError {
    /// The app credentials themselves (an app id, a private key, a webhook
    /// secret) are missing or malformed.
    #[error("github app misconfigured: {0}")]
    Config(String),
    /// Authenticating as the app, or exchanging its JWT for an
    /// installation access token, failed.
    #[error("couldn't authenticate with github: {0}")]
    Auth(String),
    /// A webhook delivery's body couldn't be parsed as the shape its own
    /// `X-GitHub-Event` header promised.
    #[error("couldn't parse webhook payload: {0}")]
    Payload(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_names_the_problem() {
        let error = GitHubError::Config("GITHUB_APP_ID is not a number".to_string());
        assert!(error.to_string().contains("GITHUB_APP_ID is not a number"));
    }

    #[test]
    fn test_auth_error_names_the_underlying_failure() {
        let error = GitHubError::Auth("401 unauthorized".to_string());
        assert!(error.to_string().contains("401 unauthorized"));
    }

    #[test]
    fn test_payload_error_names_the_parse_failure() {
        let error = GitHubError::Payload("missing field `issue`".to_string());
        assert!(error.to_string().contains("missing field `issue`"));
    }
}
