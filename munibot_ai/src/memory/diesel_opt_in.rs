use munibot_core::db::{DbPool, operations::ai};

use crate::{memory::opt_in::MemoryOptIn, types::AiError};

/// Database failures surface as [`AiError::Other`], the same convention every
/// diesel-backed store in this module uses.
fn db_error(error: impl std::fmt::Display) -> AiError {
    AiError::Other(format!("the database had trouble :< {error}"))
}

/// A [`MemoryOptIn`] backed by MySQL through `diesel-async`.
pub struct DieselMemoryOptIn {
    pool: DbPool,
}

impl DieselMemoryOptIn {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl MemoryOptIn for DieselMemoryOptIn {
    async fn is_opted_in(&self, user_id: u64) -> Result<bool, AiError> {
        let settings = ai::get_user_settings(&self.pool, user_id as i64)
            .await
            .map_err(db_error)?;
        // no row at all means the setting has never been touched, which is the
        // same as "off" - memory is opt-in, never assumed
        Ok(settings.is_some_and(|settings| settings.memory_opt_in))
    }

    async fn set_opted_in(&self, user_id: u64, opted_in: bool) -> Result<(), AiError> {
        ai::set_memory_opt_in(&self.pool, user_id as i64, opted_in)
            .await
            .map_err(db_error)?;
        Ok(())
    }
}
