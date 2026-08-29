use async_trait::async_trait;
use munibot_core::db::{DbPool, models::NewAiSafetyEvent, operations::ai};

use crate::safety::{SafetyEvent, SafetyEventAuditor};

/// A [`SafetyEventAuditor`] backed by MySQL through `diesel-async`.
pub struct DieselSafetyEventAuditor {
    pool: DbPool,
}

impl DieselSafetyEventAuditor {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SafetyEventAuditor for DieselSafetyEventAuditor {
    async fn record(&self, event: SafetyEvent) {
        let row = NewAiSafetyEvent {
            event_type: event.event_type.as_key().to_string(),
            scope_type: event.scope.scope_type().to_string(),
            scope_id: event.scope.scope_id(),
            reason: event.reason,
            content_hash: event.content_hash,
            created_at: chrono::Utc::now().naive_utc(),
        };

        if let Err(error) = ai::record_safety_event(&self.pool, row).await {
            tracing::warn!(%error, "couldn't record a safety event");
        }
    }
}
