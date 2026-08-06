use std::sync::Arc;

use chrono::Utc;

use crate::limits::{Scope, SpendCapError, SpendCapPolicies, SpendCapPolicy, SpendCapStore};

/// How much of a cap has to be spent before it logs a warning, so an
/// operator sees a scope approaching its limit before it actually refuses
/// anything.
const WARN_AT_RATIO: f64 = 0.8;

/// Tracks spend per scope against configured caps, refusing new turns once
/// one is fully spent while letting anything already running finish - the
/// "kill switch" only ever applies at the point a turn would otherwise
/// start, in [`Self::check`], never mid-turn.
pub struct SpendCapEnforcer {
    store: Arc<dyn SpendCapStore>,
    policies: SpendCapPolicies,
}

impl SpendCapEnforcer {
    pub fn new(store: Arc<dyn SpendCapStore>, policies: SpendCapPolicies) -> Self {
        Self { store, policies }
    }

    /// The policy for `scope`'s kind, or `None` for a guild - spend caps
    /// only ever apply per user and globally, see [`SpendCapPolicies`]'s own
    /// doc comment for why.
    fn policy_for(&self, scope: Scope) -> Option<&SpendCapPolicy> {
        match scope {
            Scope::User(_) => Some(&self.policies.user),
            Scope::Global => Some(&self.policies.global),
            Scope::Guild(_) => None,
        }
    }

    /// Checks whether `scope` still has room under its configured cap, to
    /// be called before the provider call.
    ///
    /// A no-op for a guild scope, or for any scope with no cap configured -
    /// see [`Self::policy_for`]. A store failure fails open (logs a warning,
    /// allows the turn): a database hiccup says nothing about whether this
    /// particular scope has actually overspent, the same reasoning
    /// [`crate::limits::RateLimiter`]'s own window check already documents.
    pub async fn check(&self, scope: Scope) -> Result<(), SpendCapError> {
        let Some(policy) = self.policy_for(scope) else {
            return Ok(());
        };
        let Some(limit_micros) = policy.limit_micros else {
            return Ok(());
        };

        let now = Utc::now();
        let existing = match self.store.get_cap(scope, &policy.period).await {
            Ok(existing) => existing,
            Err(error) => {
                tracing::warn!(%error, "couldn't check a spend cap; allowing the turn");
                return Ok(());
            }
        };

        let current_micros = match existing {
            Some(cap) if cap.reset_at > now => cap.current_micros,
            _ => {
                let reset_at = now + policy.duration;
                if let Err(error) = self
                    .store
                    .upsert_cap(scope, &policy.period, limit_micros, 0, reset_at)
                    .await
                {
                    tracing::warn!(%error, "couldn't roll a spend cap over; allowing the turn");
                    return Ok(());
                }
                0
            }
        };

        if current_micros >= limit_micros {
            let reset_at = existing
                .map(|cap| cap.reset_at)
                .unwrap_or(now + policy.duration);
            return Err(SpendCapError {
                reset_at: reset_at.to_rfc3339(),
            });
        }

        if limit_micros > 0 && (current_micros as f64 / limit_micros as f64) >= WARN_AT_RATIO {
            tracing::warn!(
                scope_type = scope.scope_type(),
                scope_id = scope.scope_id(),
                current_micros,
                limit_micros,
                "a spend cap has reached 80% or more of its limit"
            );
        }

        Ok(())
    }

    /// Records spend actually incurred by a turn, once it has finished -
    /// there is no way to know a turn's own cost before the model answers,
    /// so this happens strictly after [`Self::check`], never as part of it.
    ///
    /// A store failure is logged and otherwise ignored, the same reasoning
    /// [`crate::limits::RateLimiter::record_tokens`] documents.
    pub async fn record_spend(&self, scope: Scope, cost_micros: i64) {
        let Some(policy) = self.policy_for(scope) else {
            return;
        };
        if policy.limit_micros.is_none() || cost_micros == 0 {
            return;
        }
        if let Err(error) = self
            .store
            .increment_spend(scope, &policy.period, cost_micros)
            .await
        {
            tracing::warn!(%error, "couldn't record spend for a spend cap");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::{limits::SpendCapRow, types::AiError};

    type CapKey = (String, Option<i64>, String);

    #[derive(Default)]
    struct FakeSpendCapStore {
        caps: Mutex<HashMap<CapKey, SpendCapRow>>,
    }

    fn key(scope: Scope, period: &str) -> CapKey {
        (
            scope.scope_type().to_string(),
            scope.scope_id(),
            period.to_string(),
        )
    }

    #[async_trait]
    impl SpendCapStore for FakeSpendCapStore {
        async fn get_cap(
            &self,
            scope: Scope,
            period: &str,
        ) -> Result<Option<SpendCapRow>, AiError> {
            Ok(self.caps.lock().unwrap().get(&key(scope, period)).copied())
        }

        async fn upsert_cap(
            &self,
            scope: Scope,
            period: &str,
            limit_micros: i64,
            current_micros: i64,
            reset_at: DateTime<Utc>,
        ) -> Result<(), AiError> {
            self.caps
                .lock()
                .unwrap()
                .insert(key(scope, period), SpendCapRow {
                    limit_micros,
                    current_micros,
                    reset_at,
                });
            Ok(())
        }

        async fn increment_spend(
            &self,
            scope: Scope,
            period: &str,
            micros: i64,
        ) -> Result<(), AiError> {
            let mut caps = self.caps.lock().unwrap();
            if let Some(cap) = caps.get_mut(&key(scope, period)) {
                cap.current_micros += micros;
            }
            Ok(())
        }
    }

    fn enforcer_with(policies: SpendCapPolicies) -> SpendCapEnforcer {
        SpendCapEnforcer::new(Arc::new(FakeSpendCapStore::default()), policies)
    }

    #[tokio::test]
    async fn test_an_unconfigured_scope_is_never_refused() {
        let enforcer = enforcer_with(SpendCapPolicies::default());
        enforcer
            .check(Scope::User(1))
            .await
            .expect("should succeed");
        enforcer.check(Scope::Global).await.expect("should succeed");
    }

    #[tokio::test]
    async fn test_a_guild_scope_is_never_checked_at_all() {
        let enforcer = enforcer_with(SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(0),
                ..SpendCapPolicy::default()
            },
            global: SpendCapPolicy {
                limit_micros: Some(0),
                ..SpendCapPolicy::default()
            },
        });
        // both user and global are already fully spent (limit 0), but a
        // guild scope should still never be refused, since it is never
        // checked against a spend cap at all
        enforcer
            .check(Scope::Guild(1))
            .await
            .expect("guild scopes are never subject to a spend cap");
    }

    #[tokio::test]
    async fn test_a_scope_is_refused_once_current_spend_reaches_the_limit() {
        let store = Arc::new(FakeSpendCapStore::default());
        let enforcer = SpendCapEnforcer::new(store.clone(), SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(1000),
                period: "monthly".to_string(),
                duration: Duration::from_secs(60 * 60),
            },
            ..SpendCapPolicies::default()
        });

        enforcer
            .check(Scope::User(1))
            .await
            .expect("should succeed before any spend");
        enforcer.record_spend(Scope::User(1), 1000).await;

        let result = enforcer.check(Scope::User(1)).await;
        assert!(result.is_err(), "reaching the limit exactly should refuse");
    }

    #[tokio::test]
    async fn test_spend_below_the_limit_is_allowed() {
        let enforcer = enforcer_with(SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(1000),
                period: "monthly".to_string(),
                duration: Duration::from_secs(60 * 60),
            },
            ..SpendCapPolicies::default()
        });

        enforcer
            .check(Scope::User(1))
            .await
            .expect("should succeed");
        enforcer.record_spend(Scope::User(1), 500).await;
        enforcer
            .check(Scope::User(1))
            .await
            .expect("half the limit should still be allowed");
    }

    #[tokio::test]
    async fn test_different_scopes_have_independent_caps() {
        let enforcer = enforcer_with(SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(100),
                period: "monthly".to_string(),
                duration: Duration::from_secs(60 * 60),
            },
            global: SpendCapPolicy {
                limit_micros: Some(100_000),
                period: "monthly".to_string(),
                duration: Duration::from_secs(60 * 60),
            },
        });

        enforcer.check(Scope::User(1)).await.unwrap();
        enforcer.record_spend(Scope::User(1), 100).await;
        assert!(enforcer.check(Scope::User(1)).await.is_err());

        enforcer
            .check(Scope::Global)
            .await
            .expect("a different scope should have its own independent cap");
    }

    #[tokio::test]
    async fn test_a_cap_that_has_reset_starts_over_rather_than_staying_refused() {
        let enforcer = enforcer_with(SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(100),
                period: "monthly".to_string(),
                duration: Duration::from_millis(10),
            },
            ..SpendCapPolicies::default()
        });

        enforcer.check(Scope::User(1)).await.unwrap();
        enforcer.record_spend(Scope::User(1), 100).await;
        assert!(enforcer.check(Scope::User(1)).await.is_err());

        tokio::time::sleep(Duration::from_millis(30)).await;

        enforcer
            .check(Scope::User(1))
            .await
            .expect("a rolled-over period should allow spend again");
    }

    #[tokio::test]
    async fn test_recording_zero_spend_is_a_no_op() {
        let enforcer = enforcer_with(SpendCapPolicies {
            user: SpendCapPolicy {
                limit_micros: Some(1),
                period: "monthly".to_string(),
                duration: Duration::from_secs(60 * 60),
            },
            ..SpendCapPolicies::default()
        });

        enforcer.record_spend(Scope::User(1), 0).await;
        enforcer
            .check(Scope::User(1))
            .await
            .expect("should still succeed");
    }
}
