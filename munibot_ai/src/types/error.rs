use std::time::Duration;

use thiserror::Error;

/// Something that went wrong talking to a model.
///
/// Everything in this crate reports failures through this type, so a caller
/// several layers up can react to one error hierarchy.
#[derive(Error, Debug)]
pub enum AiError {
    /// The provider had a transient problem: an outage, a connection reset, a
    /// 5xx response. Worth retrying.
    #[error("the model provider had trouble :< {0}")]
    Provider(String),

    /// The provider flatly rejected the request: a 4xx response other than 429,
    /// a bad API key, a malformed request body. Never worth retrying
    /// unchanged - the request itself needs fixing.
    #[error("the model provider rejected the request :< {0}")]
    Rejected(String),

    /// The provider asked us to slow down.
    #[error("rate limited by the model provider :< try again in {retry_after:?}")]
    RateLimited {
        /// How long to wait before retrying, when the provider said.
        retry_after: Option<Duration>,
    },

    /// A provider was misconfigured, such as an unknown provider name or a
    /// missing api key.
    #[error("something's misconfigured :< {0}")]
    Config(String),

    /// Anything else.
    #[error("something went wrong :< {0}")]
    Other(String),
}

impl AiError {
    /// Returns `true` if retrying the same request has a reasonable chance of
    /// succeeding.
    ///
    /// A rate limit or a provider-side outage is worth retrying, but a bad
    /// configuration or a rejected request is not, and retrying it would just
    /// repeat the failure.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Provider(_) | Self::RateLimited { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_errors_are_transient() {
        assert!(
            AiError::Provider("connection reset".to_string()).is_transient(),
            "a provider-side failure is usually worth retrying"
        );
    }

    #[test]
    fn test_rate_limited_is_transient() {
        assert!(
            AiError::RateLimited {
                retry_after: Some(Duration::from_secs(5))
            }
            .is_transient(),
            "a rate limit is exactly what retrying is for"
        );
    }

    #[test]
    fn test_rejected_is_not_transient() {
        assert!(
            !AiError::Rejected("bad api key".to_string()).is_transient(),
            "a flat rejection needs a fixed request, not a repeated one"
        );
    }

    #[test]
    fn test_config_errors_are_not_transient() {
        assert!(
            !AiError::Config("missing ANTHROPIC_API_KEY".to_string()).is_transient(),
            "retrying a misconfiguration would just repeat the same failure"
        );
    }

    #[test]
    fn test_error_messages_are_lowercase_and_friendly() {
        let message = AiError::Tool("timed out".to_string()).to_string();
        assert!(
            message.chars().next().is_some_and(|c| !c.is_uppercase()),
            "error messages should start lowercase, got {message:?}"
        );
        assert!(
            message.contains(":<"),
            "error messages should carry the house emoticon style, got {message:?}"
        );
    }

    #[test]
    fn test_anyhow_errors_convert_to_other() {
        let source = anyhow::anyhow!("disk full");
        let error: AiError = source.into();
        assert!(
            matches!(error, AiError::Other(_)),
            "an anyhow error should land in Other rather than being dropped"
        );
    }
}
