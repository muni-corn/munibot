use async_trait::async_trait;
use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, models::NewAiSpendCap, operations::ai};

use crate::{limits::Scope, types::AiError};

/// A scope's spend cap for one period, as read from storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpendCapRow {
    pub limit_micros: i64,
    pub current_micros: i64,
    pub reset_at: DateTime<Utc>,
}

/// Where [`crate::limits::SpendCapEnforcer`] reads and writes spend caps.
///
/// A trait rather than [`DieselSpendCapStore`] used directly, the same
/// reasoning [`crate::limits::RateLimitStore`] exists for at all.
#[async_trait]
pub trait SpendCapStore: Send + Sync {
    /// The current cap for `scope`'s `period`, if one has ever been created.
    async fn get_cap(&self, scope: Scope, period: &str) -> Result<Option<SpendCapRow>, AiError>;

    /// Creates or wholesale replaces `scope`'s cap for `period` - used both
    /// the first time a scope is checked and to roll a cap over once its
    /// own `reset_at` has passed.
    async fn upsert_cap(
        &self,
        scope: Scope,
        period: &str,
        limit_micros: i64,
        current_micros: i64,
        reset_at: DateTime<Utc>,
    ) -> Result<(), AiError>;

    /// Adds to `scope`'s spend within its current period.
    async fn increment_spend(&self, scope: Scope, period: &str, micros: i64)
    -> Result<(), AiError>;
}

/// A [`SpendCapStore`] backed by MySQL through `diesel-async`.
pub struct DieselSpendCapStore {
    pool: DbPool,
}

impl DieselSpendCapStore {
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
impl SpendCapStore for DieselSpendCapStore {
    async fn get_cap(&self, scope: Scope, period: &str) -> Result<Option<SpendCapRow>, AiError> {
        let row = ai::get_spend_cap(&self.pool, scope.scope_type(), scope.scope_id(), period)
            .await
            .map_err(db_error)?;

        Ok(row.map(|row| SpendCapRow {
            limit_micros: row.limit_micros,
            current_micros: row.current_micros,
            reset_at: DateTime::<Utc>::from_naive_utc_and_offset(row.reset_at, Utc),
        }))
    }

    async fn upsert_cap(
        &self,
        scope: Scope,
        period: &str,
        limit_micros: i64,
        current_micros: i64,
        reset_at: DateTime<Utc>,
    ) -> Result<(), AiError> {
        ai::upsert_spend_cap(&self.pool, NewAiSpendCap {
            scope_type: scope.scope_type().to_string(),
            scope_id: scope.scope_id(),
            period: period.to_string(),
            limit_micros,
            current_micros,
            reset_at: reset_at.naive_utc(),
        })
        .await
        .map_err(db_error)
    }

    async fn increment_spend(
        &self,
        scope: Scope,
        period: &str,
        micros: i64,
    ) -> Result<(), AiError> {
        ai::increment_spend(
            &self.pool,
            scope.scope_type(),
            scope.scope_id(),
            period,
            micros,
        )
        .await
        .map_err(db_error)
    }
}
