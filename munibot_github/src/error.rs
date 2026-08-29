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
}
