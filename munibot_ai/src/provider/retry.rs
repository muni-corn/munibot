use std::time::Duration;

use async_trait::async_trait;
use backon::{Backoff, BackoffBuilder, ExponentialBuilder};
use futures::stream::BoxStream;

use crate::{
    provider::Provider,
    types::{AiError, CompletionRequest, CompletionResponse, StreamEvent},
};

/// How eagerly to retry a failed request.
///
/// Deliberately conservative by default: a persona misconfigured against a
/// genuinely broken provider should fail fast, not hang a Discord reply for a
/// minute while it retries.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Total attempts, including the first. `1` means no retries at all.
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
    /// Whether to randomize each delay within `(0, computed_delay)`, to avoid
    /// many callers retrying in lockstep after a shared outage.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    fn backoff(&self) -> impl Backoff {
        let mut builder = ExponentialBuilder::default()
            .with_min_delay(self.base_delay)
            .with_max_delay(self.max_delay)
            // this only bounds the exponential schedule; max_attempts is enforced separately
            // below, since it must also cover attempts spent honouring a provider's retry_after
            .with_max_times(self.max_attempts.saturating_sub(1));

        if self.jitter {
            builder = builder.with_jitter();
        }

        builder.build()
    }
}

/// Wraps any [`Provider`] with [`RetryPolicy`], retrying only when the failure
/// is [`AiError::is_transient`].
///
/// Honours [`AiError::RateLimited`]'s `retry_after` when the provider supplies
/// one, sleeping for exactly that long instead of the computed backoff delay.
/// In practice no provider we target currently supplies one - see finding 10 in
/// `docs/notes/ai-preflight-findings.md` - so this path is presently
/// unreachable in production, but the interface is correct for whenever it is.
pub struct RetryingProvider<P> {
    inner: P,
    policy: RetryPolicy,
}

impl<P: Provider> RetryingProvider<P> {
    pub fn new(inner: P, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    /// Decides how long to wait before the next attempt, given a transient
    /// error and the exponential schedule - or `None` if the schedule has
    /// nothing left to give.
    ///
    /// A rate limit's own `retry_after` always wins over the computed delay,
    /// when the provider supplies one.
    fn next_delay(error: &AiError, backoff: &mut impl Backoff) -> Option<Duration> {
        match error {
            AiError::RateLimited {
                retry_after: Some(delay),
            } => Some(*delay),
            _ => backoff.next(),
        }
    }
}

#[async_trait]
impl<P: Provider> Provider for RetryingProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AiError> {
        let mut backoff = self.policy.backoff();
        let mut attempts_made = 0usize;

        loop {
            attempts_made += 1;
            match self.inner.complete(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => {
                    if !error.is_transient() || attempts_made >= self.policy.max_attempts {
                        return Err(error);
                    }
                    match Self::next_delay(&error, &mut backoff) {
                        Some(delay) => tokio::time::sleep(delay).await,
                        None => return Err(error),
                    }
                }
            }
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, AiError>>, AiError> {
        // a stream is retried as a whole on a failure to *start* it; once events are
        // flowing, a mid-stream error is surfaced to the caller rather than
        // silently restarting a partial response, which could otherwise
        // duplicate content already delivered
        let mut backoff = self.policy.backoff();
        let mut attempts_made = 0usize;

        loop {
            attempts_made += 1;
            match self.inner.stream(request.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    if !error.is_transient() || attempts_made >= self.policy.max_attempts {
                        return Err(error);
                    }
                    match Self::next_delay(&error, &mut backoff) {
                        Some(delay) => tokio::time::sleep(delay).await,
                        None => return Err(error),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    /// A policy with negligible delays, so retry tests run in milliseconds
    /// rather than needing tokio's paused-time test runtime.
    fn fast_policy(max_attempts: usize) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            jitter: false,
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest::new(
            crate::types::ModelRef::new("anthropic", "claude-opus-5"),
            vec![crate::types::Message::user("hi")].into(),
        )
    }

    #[tokio::test]
    async fn test_succeeds_immediately_without_retrying() {
        let mock = MockProvider::new().respond_text("hi");
        let provider = RetryingProvider::new(mock, fast_policy(4));

        let response = provider.complete(request()).await.expect("should succeed");
        assert_eq!(response.text(), "hi");
    }

    #[tokio::test]
    async fn test_retries_a_transient_error_until_it_succeeds() {
        let mock = MockProvider::new()
            .respond_error(AiError::Provider("connection reset".to_string()))
            .respond_error(AiError::Provider("connection reset".to_string()))
            .respond_text("finally");
        let provider = RetryingProvider::new(mock, fast_policy(4));

        let response = provider
            .complete(request())
            .await
            .expect("should eventually succeed");
        assert_eq!(response.text(), "finally");
    }

    #[tokio::test]
    async fn test_gives_up_after_max_attempts() {
        let mock = MockProvider::new()
            .respond_error(AiError::Provider("down".to_string()))
            .respond_error(AiError::Provider("down".to_string()))
            .respond_error(AiError::Provider("down".to_string()));
        let provider = RetryingProvider::new(mock, fast_policy(3));

        let result = provider.complete(request()).await;
        assert!(
            result.is_err(),
            "exhausting every attempt should surface the last error"
        );
    }

    #[tokio::test]
    async fn test_does_not_retry_a_permanent_rejection() {
        // only one response is scripted; MockProvider panics if asked for a second,
        // which is exactly the assertion that a permanent rejection was not
        // retried
        let mock = MockProvider::new().respond_error(AiError::Rejected("bad api key".to_string()));
        let provider = RetryingProvider::new(mock, fast_policy(4));

        let result = provider.complete(request()).await;
        assert!(
            result.is_err(),
            "a permanent rejection should surface immediately"
        );
    }

    #[tokio::test]
    async fn test_max_attempts_of_one_never_retries() {
        let mock = MockProvider::new().respond_error(AiError::Provider("down".to_string()));
        let provider = RetryingProvider::new(mock, fast_policy(1));

        let result = provider.complete(request()).await;
        assert!(
            result.is_err(),
            "a single-attempt policy should not retry even a transient error"
        );
    }

    #[tokio::test]
    async fn test_name_delegates_to_the_inner_provider() {
        let mock = MockProvider::new().named("anthropic");
        let provider = RetryingProvider::new(mock, fast_policy(1));
        assert_eq!(provider.name(), "anthropic");
    }

    #[tokio::test]
    async fn test_rate_limited_with_retry_after_is_honoured_over_the_computed_backoff() {
        // the delay itself cannot be asserted on without a paused clock, but this
        // proves the rate-limited path is retried like any other transient
        // error rather than treated as permanent, and completes promptly since
        // retry_after here is negligible
        let mock = MockProvider::new()
            .respond_error(AiError::RateLimited {
                retry_after: Some(Duration::from_millis(1)),
            })
            .respond_text("ok");
        let provider = RetryingProvider::new(mock, fast_policy(3));

        let response = provider
            .complete(request())
            .await
            .expect("should retry and succeed");
        assert_eq!(response.text(), "ok");
    }
}
