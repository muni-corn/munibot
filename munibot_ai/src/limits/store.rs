use async_trait::async_trait;
use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, operations::ai};

use crate::{limits::Scope, types::AiError};

/// A scope's rate limit window, as read from storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimitWindow {
    pub window_start: DateTime<Utc>,
    pub request_count: u32,
    pub token_count: u64,
}

/// Where [`crate::limits::RateLimiter`] reads and writes rate limit windows.
///
/// A trait rather than [`DieselRateLimitStore`] used directly, so a unit
/// test can substitute one that never touches a database - the same
/// reasoning `crate::provider::MockProvider` exists for at all.
#[async_trait]
pub trait RateLimitStore: Send + Sync {
    /// The current window for `scope`, if one has ever been started.
    async fn get_window(&self, scope: Scope) -> Result<Option<RateLimitWindow>, AiError>;

    /// Starts a fresh window for `scope`, replacing whatever existed.
    async fn reset_window(
        &self,
        scope: Scope,
        window_start: DateTime<Utc>,
        request_count: u32,
        token_count: u64,
    ) -> Result<(), AiError>;

    /// Adds to `scope`'s counters within its current window.
    async fn increment(&self, scope: Scope, requests: u32, tokens: u64) -> Result<(), AiError>;
}

/// A [`RateLimitStore`] backed by MySQL through `diesel-async`.
pub struct DieselRateLimitStore {
    pool: DbPool,
}

impl DieselRateLimitStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// Database failures surface as [`AiError::Other`] rather than a dedicated
/// variant - the same reasoning `crate::memory::diesel_store`'s own
/// `db_error` helper documents: nothing above this layer can act
/// differently on a connection error than on any other unexpected failure.
fn db_error(error: impl std::fmt::Display) -> AiError {
    AiError::Other(format!("the database had trouble :< {error}"))
}

#[async_trait]
impl RateLimitStore for DieselRateLimitStore {
    async fn get_window(&self, scope: Scope) -> Result<Option<RateLimitWindow>, AiError> {
        let row = ai::get_rate_limit(&self.pool, scope.scope_type(), scope.scope_id())
            .await
            .map_err(db_error)?;

        Ok(row.map(|row| RateLimitWindow {
            window_start: DateTime::<Utc>::from_naive_utc_and_offset(row.window_start, Utc),
            request_count: row.request_count as u32,
            token_count: row.token_count as u64,
        }))
    }

    async fn reset_window(
        &self,
        scope: Scope,
        window_start: DateTime<Utc>,
        request_count: u32,
        token_count: u64,
    ) -> Result<(), AiError> {
        ai::reset_rate_limit_window(
            &self.pool,
            scope.scope_type(),
            scope.scope_id(),
            window_start.naive_utc(),
            request_count as i32,
            token_count as i64,
        )
        .await
        .map_err(db_error)
    }

    async fn increment(&self, scope: Scope, requests: u32, tokens: u64) -> Result<(), AiError> {
        ai::increment_rate_limit(
            &self.pool,
            scope.scope_type(),
            scope.scope_id(),
            requests as i32,
            tokens as i64,
        )
        .await
        .map_err(db_error)
    }
}
