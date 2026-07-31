use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::types::AiError;

/// One thing a user has asked munibot to remember about them.
#[derive(Clone, Debug, PartialEq)]
pub struct Memory {
    pub key: String,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

/// Stores a user's own, explicitly opted-in long-term memory.
///
/// Distinct from [`crate::memory::SessionStore`], which is one conversation's
/// history: this is a person's memory, following them across every
/// conversation and every platform, keyed on the internal `users.id`.
///
/// Opt-in gating is deliberately **not** this trait's job. It is layered on
/// top of an implementation as a decorator instead, so every implementation -
/// including a future in-memory one for tests - automatically gets the same
/// gating with no chance of a new implementation forgetting it.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Every memory a user has recorded, in no particular guaranteed order.
    async fn list(&self, user_id: u64) -> Result<Vec<Memory>, AiError>;

    /// Records a fact under `key`, replacing any existing value for that key.
    ///
    /// An implementation may refuse a genuinely new key past some per-user
    /// cap, returning a recoverable [`AiError`] - updating an existing key's
    /// value should always be allowed, since it does not grow how many a
    /// user has.
    async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError>;

    /// Forgets one specific memory. Not an error if it never existed.
    async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError>;

    /// Forgets everything a user has ever recorded.
    async fn wipe(&self, user_id: u64) -> Result<(), AiError>;
}
