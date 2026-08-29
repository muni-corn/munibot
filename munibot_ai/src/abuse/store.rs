use async_trait::async_trait;
use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, operations::ai};

use crate::{limits::Scope, types::AiError};

/// A scope's abuse-cooldown row, as read from storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbuseCooldownRow {
    pub strike_count: u32,
    pub cooldown_until: DateTime<Utc>,
}

/// Where [`crate::abuse::AbuseDetector`] reads and writes cooldown state.
///
/// A trait rather than [`DieselAbuseStore`] used directly, so a unit test
/// can substitute one that never touches a database - the same reasoning
/// [`crate::limits::RateLimitStore`] exists at all.
#[async_trait]
pub trait AbuseStore: Send + Sync {
    /// The current cooldown row for `scope`, if it has ever tripped before.
    async fn get(&self, scope: Scope) -> Result<Option<AbuseCooldownRow>, AiError>;

    /// Records a fresh strike for `scope`: its new strike count, how long
    /// it is now cooling down for, and why - a short, stable reason string
    /// (see [`crate::abuse::AbuseSignal::reason`]), never message content.
    async fn record_strike(
        &self,
        scope: Scope,
        strike_count: u32,
        cooldown_until: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), AiError>;
}

/// An [`AbuseStore`] backed by MySQL through `diesel-async`.
pub struct DieselAbuseStore {
    pool: DbPool,
}

impl DieselAbuseStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// Database failures surface as [`AiError::Other`] rather than a dedicated
/// variant - the same reasoning `crate::limits::store`'s own `db_error`
/// helper documents.
fn db_error(error: impl std::fmt::Display) -> AiError {
    AiError::Other(format!("the database had trouble :< {error}"))
}

#[async_trait]
impl AbuseStore for DieselAbuseStore {
    async fn get(&self, scope: Scope) -> Result<Option<AbuseCooldownRow>, AiError> {
        let row = ai::get_abuse_cooldown(&self.pool, scope.scope_type(), scope.scope_id())
            .await
            .map_err(db_error)?;

        Ok(row.map(|row| AbuseCooldownRow {
            strike_count: row.strike_count as u32,
            cooldown_until: DateTime::<Utc>::from_naive_utc_and_offset(row.cooldown_until, Utc),
        }))
    }

    async fn record_strike(
        &self,
        scope: Scope,
        strike_count: u32,
        cooldown_until: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), AiError> {
        ai::upsert_abuse_cooldown(
            &self.pool,
            scope.scope_type(),
            scope.scope_id(),
            strike_count as i32,
            cooldown_until.naive_utc(),
            Utc::now().naive_utc(),
            reason,
        )
        .await
        .map_err(db_error)
    }
}
