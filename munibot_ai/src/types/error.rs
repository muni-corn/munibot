use std::time::Duration;

use thiserror::Error;

/// Something that went wrong somewhere in the ai harness.
///
/// Every `munibot_ai_*` crate reports failures through this type, so a caller
/// several layers up (a Discord handler, a pipeline executor) can react to one
/// error hierarchy rather than a different one per crate.
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

    /// A configured budget (iterations, tokens, wall clock, cost) was
    /// exhausted.
    #[error("hit a budget limit :< {limit}")]
    BudgetExceeded {
        /// A human-readable description of which limit was hit, since a persona
        /// can carry several.
        limit: String,
    },

    /// The turn was refused for a reason other than budget: an abuse
    /// cooldown (`crate::abuse::AbuseDetector`), or a moderation check that
    /// failed closed. Distinct from [`Self::BudgetExceeded`] since a caller
    /// may reasonably want to react differently - neither of these is about
    /// cost, and unlike a budget limit, both are inherently temporary or
    /// content-specific rather than something retrying the same request
    /// unchanged could ever get past.
    #[error("that got refused :< {0}")]
    Refused(String),

    /// A tool failed in a way the model cannot recover from by retrying.
    #[error("a tool failed :< {0}")]
    Tool(String),

    /// A handoff or tool call did not match its expected schema, even after
    /// retries.
    #[error("that didn't match the expected shape :< {0}")]
    SchemaViolation(String),

    /// The turn was cancelled before it finished.
    #[error("cancelled before finishing :<")]
    Cancelled,

    /// A persona, provider, or tool was misconfigured.
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
    /// The retry layer in the `provider` module consumes this directly: a rate
    /// limit or a provider-side outage is worth retrying, but a bad
    /// configuration or a cancelled turn is not, and retrying it would just
    /// repeat the failure.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Provider(_) | Self::RateLimited { .. })
    }
}

impl From<anyhow::Error> for AiError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
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
    fn test_cancelled_is_not_transient() {
        assert!(
            !AiError::Cancelled.is_transient(),
            "a cancelled turn should not be retried on the caller's behalf"
        );
    }

    #[test]
    fn test_schema_violation_is_not_transient() {
        // schema failures are handled by the harness's own retry-with-feedback loop,
        // not by the provider-level retry policy
        assert!(
            !AiError::SchemaViolation("missing required field `key`".to_string()).is_transient(),
            "schema violations need a corrected request, not a repeated one"
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
