use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, operations::ai};

use crate::{
    memory::user_memory::{Memory, MemoryStore},
    types::AiError,
};

/// The most memories one user may have recorded at once.
///
/// Prevents unbounded growth. Recording a genuinely new key past this limit
/// is refused, so the person doing the remembering decides what to forget -
/// munibot never silently evicts a memory they might still care about.
/// Updating an existing key's value is always allowed regardless of this
/// limit, since it does not grow how many a user has.
const MAX_MEMORIES_PER_USER: i64 = 100;

/// Database failures surface as [`AiError::Other`], the same convention
/// [`crate::memory::DieselSessionStore`] uses and for the same reason:
/// nothing above this layer can act differently on a connection error than on
/// any other unexpected failure.
fn db_error(error: impl std::fmt::Display) -> AiError {
    AiError::Other(format!("the database had trouble :< {error}"))
}

/// A [`MemoryStore`] backed by MySQL through `diesel-async`.
pub struct DieselMemoryStore {
    pool: DbPool,
}

impl DieselMemoryStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl MemoryStore for DieselMemoryStore {
    async fn list(&self, user_id: u64) -> Result<Vec<Memory>, AiError> {
        let rows = ai::list_memories(&self.pool, user_id as i64)
            .await
            .map_err(db_error)?;

        Ok(rows
            .into_iter()
            .map(|row| Memory {
                key: row.key,
                value: row.value,
                updated_at: DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at, Utc),
            })
            .collect())
    }

    async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError> {
        let user_id = user_id as i64;

        // an update to an existing key never grows the count, so only a genuinely
        // new key needs to be checked against the cap
        let existing = ai::get_memory(&self.pool, user_id, key)
            .await
            .map_err(db_error)?;
        if existing.is_none() {
            let count = ai::count_memories(&self.pool, user_id)
                .await
                .map_err(db_error)?;
            if count >= MAX_MEMORIES_PER_USER {
                return Err(AiError::Config(format!(
                    "you've already got {MAX_MEMORIES_PER_USER} memories saved :< forget \
                     something first"
                )));
            }
        }

        ai::upsert_memory(&self.pool, user_id, key, value)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError> {
        ai::forget_memory(&self.pool, user_id as i64, key)
            .await
            .map_err(db_error)
    }

    async fn wipe(&self, user_id: u64) -> Result<(), AiError> {
        ai::wipe_memories(&self.pool, user_id as i64)
            .await
            .map_err(db_error)
    }
}
