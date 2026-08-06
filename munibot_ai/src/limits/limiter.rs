use std::sync::Arc;

use chrono::Utc;

use crate::limits::{
    ConcurrencyGuard, RateLimitError, RateLimitPolicy, RateLimitStore, Scope, ScopePolicies,
    concurrency::ConcurrencyTracker,
};

/// A sliding-window rate limiter over the database, with a small in-memory
/// cache for the one dimension that benefits from being purely in-memory:
/// how many turns are in flight for a scope right now (see
/// [`crate::limits::ConcurrencyGuard`]'s own doc comment for why that piece
/// specifically never touches storage). Request and token counts are always
/// read fresh from `store` - correctness under concurrent access matters
/// more here than shaving one query, and a database round trip costs
/// nothing next to the model call it gates.
pub struct RateLimiter {
    store: Arc<dyn RateLimitStore>,
    policies: ScopePolicies,
    concurrency: ConcurrencyTracker,
}

impl RateLimiter {
    pub fn new(store: Arc<dyn RateLimitStore>, policies: ScopePolicies) -> Self {
        Self {
            store,
            policies,
            concurrency: ConcurrencyTracker::default(),
        }
    }

    fn policy_for(&self, scope: Scope) -> RateLimitPolicy {
        match scope {
            Scope::User(_) => self.policies.user,
            Scope::Guild(_) => self.policies.guild,
            Scope::Global => self.policies.global,
        }
    }

    /// Checks and reserves capacity for one new turn at `scope`, to be
    /// called before the provider call.
    ///
    /// Returns a guard that must be held for the turn's own lifetime and
    /// released (dropped) once it finishes, so its concurrency slot frees
    /// up. Checks concurrency first: it is the cheapest check, entirely
    /// in-memory, and the one most likely to be hit by a genuine runaway
    /// loop, so there is no reason to pay for a database round trip before
    /// ruling it out.
    pub async fn check(&self, scope: Scope) -> Result<ConcurrencyGuard, RateLimitError> {
        let policy = self.policy_for(scope);

        let guard = match policy.max_concurrent_turns {
            Some(max) => self
                .concurrency
                .try_acquire(scope, max)
                .ok_or(RateLimitError::TooManyConcurrentTurns)?,
            None => ConcurrencyGuard::inert(),
        };

        if policy.max_requests.is_some() || policy.max_tokens.is_some() {
            self.check_window(scope, &policy).await?;
        }

        Ok(guard)
    }

    /// Records tokens actually spent by a turn, once it has finished -
    /// there is no way to know a turn's own token cost before the model
    /// answers, so this happens strictly after [`Self::check`], never as
    /// part of it.
    ///
    /// A store failure is logged and otherwise ignored: a turn that already
    /// finished has nothing to gain from its own bookkeeping failing it
    /// retroactively.
    pub async fn record_tokens(&self, scope: Scope, tokens: u64) {
        if self.policy_for(scope).max_tokens.is_none() || tokens == 0 {
            return;
        }
        if let Err(error) = self.store.increment(scope, 0, tokens).await {
            tracing::warn!(%error, "couldn't record token usage for rate limiting");
        }
    }

    async fn check_window(
        &self,
        scope: Scope,
        policy: &RateLimitPolicy,
    ) -> Result<(), RateLimitError> {
        let now = Utc::now();

        let existing = match self.store.get_window(scope).await {
            Ok(existing) => existing,
            // fails open: a database hiccup says nothing about whether this
            // particular scope has actually been abusive, and refusing every
            // signed-in user over a transient error would be a far worse
            // outcome than letting a few extra turns through
            Err(error) => {
                tracing::warn!(%error, "couldn't check the rate limit window; allowing the turn");
                return Ok(());
            }
        };

        let (window_start, request_count, token_count) = match existing {
            Some(window)
                if now
                    .signed_duration_since(window.window_start)
                    .to_std()
                    .unwrap_or_default()
                    < policy.window =>
            {
                (
                    window.window_start,
                    window.request_count,
                    window.token_count,
                )
            }
            _ => {
                if let Err(error) = self.store.reset_window(scope, now, 0, 0).await {
                    tracing::warn!(%error, "couldn't reset the rate limit window; allowing the turn");
                    return Ok(());
                }
                (now, 0, 0)
            }
        };

        let retry_after = || {
            let remaining = (window_start + policy.window) - now;
            humantime::format_duration(remaining.to_std().unwrap_or_default()).to_string()
        };

        if let Some(max) = policy.max_requests
            && request_count >= max
        {
            return Err(RateLimitError::TooManyRequests {
                retry_after: retry_after(),
            });
        }
        if let Some(max) = policy.max_tokens
            && token_count >= max
        {
            return Err(RateLimitError::TooManyTokens {
                retry_after: retry_after(),
            });
        }

        if let Err(error) = self.store.increment(scope, 1, 0).await {
            tracing::warn!(%error, "couldn't record a rate-limited request; continuing anyway");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::limits::RateLimitWindow;

    /// A [`RateLimitStore`] over an in-memory map, so these tests never
    /// touch a database.
    #[derive(Default)]
    struct FakeRateLimitStore {
        windows: Mutex<HashMap<(String, Option<i64>), RateLimitWindow>>,
    }

    fn key(scope: Scope) -> (String, Option<i64>) {
        (scope.scope_type().to_string(), scope.scope_id())
    }

    #[async_trait]
    impl RateLimitStore for FakeRateLimitStore {
        async fn get_window(
            &self,
            scope: Scope,
        ) -> Result<Option<RateLimitWindow>, crate::types::AiError> {
            Ok(self.windows.lock().unwrap().get(&key(scope)).copied())
        }

        async fn reset_window(
            &self,
            scope: Scope,
            window_start: DateTime<Utc>,
            request_count: u32,
            token_count: u64,
        ) -> Result<(), crate::types::AiError> {
            self.windows
                .lock()
                .unwrap()
                .insert(key(scope), RateLimitWindow {
                    window_start,
                    request_count,
                    token_count,
                });
            Ok(())
        }

        async fn increment(
            &self,
            scope: Scope,
            requests: u32,
            tokens: u64,
        ) -> Result<(), crate::types::AiError> {
            let mut windows = self.windows.lock().unwrap();
            let window = windows.entry(key(scope)).or_insert(RateLimitWindow {
                window_start: Utc::now(),
                request_count: 0,
                token_count: 0,
            });
            window.request_count += requests;
            window.token_count += tokens;
            Ok(())
        }
    }

    fn limiter_with(policies: ScopePolicies) -> RateLimiter {
        RateLimiter::new(Arc::new(FakeRateLimitStore::default()), policies)
    }

    #[tokio::test]
    async fn test_an_unlimited_scope_is_never_refused() {
        let limiter = limiter_with(ScopePolicies::default());
        for _ in 0..10 {
            limiter.check(Scope::User(1)).await.expect("should succeed");
        }
    }

    #[tokio::test]
    async fn test_a_request_limit_refuses_once_exceeded() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_requests: Some(2),
                window: Duration::from_secs(60),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        limiter
            .check(Scope::User(1))
            .await
            .expect("first should succeed");
        limiter
            .check(Scope::User(1))
            .await
            .expect("second should succeed");
        let result = limiter.check(Scope::User(1)).await;
        assert!(matches!(
            result,
            Err(RateLimitError::TooManyRequests { .. })
        ));
    }

    #[tokio::test]
    async fn test_a_token_limit_refuses_once_recorded_usage_exceeds_it() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_tokens: Some(100),
                window: Duration::from_secs(60),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        limiter
            .check(Scope::User(1))
            .await
            .expect("should succeed before any usage");
        limiter.record_tokens(Scope::User(1), 150).await;

        let result = limiter.check(Scope::User(1)).await;
        assert!(matches!(result, Err(RateLimitError::TooManyTokens { .. })));
    }

    #[tokio::test]
    async fn test_a_concurrency_limit_refuses_a_second_turn_still_in_flight() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_concurrent_turns: Some(1),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        let _first = limiter.check(Scope::User(1)).await.expect("should succeed");
        let result = limiter.check(Scope::User(1)).await;
        assert!(matches!(
            result,
            Err(RateLimitError::TooManyConcurrentTurns)
        ));
    }

    #[tokio::test]
    async fn test_releasing_a_concurrency_guard_allows_another_turn() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_concurrent_turns: Some(1),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        let first = limiter.check(Scope::User(1)).await.expect("should succeed");
        drop(first);

        limiter
            .check(Scope::User(1))
            .await
            .expect("should succeed once the first is released");
    }

    #[tokio::test]
    async fn test_different_scopes_have_independent_limits() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_requests: Some(1),
                window: Duration::from_secs(60),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        limiter
            .check(Scope::User(1))
            .await
            .expect("first user should succeed");
        assert!(
            limiter.check(Scope::User(1)).await.is_err(),
            "the same user should now be refused"
        );
        limiter
            .check(Scope::User(2))
            .await
            .expect("a different user should be entirely unaffected");
    }

    #[tokio::test]
    async fn test_a_window_that_has_expired_resets_rather_than_staying_refused() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_requests: Some(1),
                window: Duration::from_millis(10),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        limiter
            .check(Scope::User(1))
            .await
            .expect("first should succeed");
        assert!(limiter.check(Scope::User(1)).await.is_err());

        tokio::time::sleep(Duration::from_millis(30)).await;

        limiter
            .check(Scope::User(1))
            .await
            .expect("a fresh window should allow another request");
    }

    #[tokio::test]
    async fn test_recording_zero_tokens_is_a_no_op() {
        let limiter = limiter_with(ScopePolicies {
            user: RateLimitPolicy {
                max_tokens: Some(1),
                window: Duration::from_secs(60),
                ..RateLimitPolicy::default()
            },
            ..ScopePolicies::default()
        });

        // recording nothing should never itself trip the limit
        limiter.record_tokens(Scope::User(1), 0).await;
        limiter
            .check(Scope::User(1))
            .await
            .expect("should still succeed");
    }
}
