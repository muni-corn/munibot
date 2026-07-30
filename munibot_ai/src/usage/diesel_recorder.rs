use async_trait::async_trait;
use munibot_core::db::{DbPool, models::NewAiUsage, operations::ai};

use crate::{
    types::AiError,
    usage::{UsageRecord, UsageRecorder},
};

/// A [`UsageRecorder`] backed by MySQL through `diesel-async`.
pub struct DieselUsageRecorder {
    pool: DbPool,
}

impl DieselUsageRecorder {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UsageRecorder for DieselUsageRecorder {
    async fn record(&self, record: UsageRecord) -> Result<(), AiError> {
        let row = NewAiUsage {
            conversation_id: record.conversation_id.map(|id| id.0 as i64),
            user_id: record.user_id.map(|id| id as i64),
            guild_id: record.guild_id.map(|id| id as i64),
            provider: record.provider,
            model: record.model,
            persona_id: record.persona_id,
            input_tokens: i64::try_from(record.usage.input_tokens).unwrap_or(i64::MAX),
            output_tokens: i64::try_from(record.usage.output_tokens).unwrap_or(i64::MAX),
            cost_micros: record.cost.0,
            iterations: i32::try_from(record.iterations).unwrap_or(i32::MAX),
            succeeded: record.succeeded,
            created_at: chrono::Utc::now().naive_utc(),
        };

        ai::record_usage(&self.pool, row)
            .await
            .map_err(|error| AiError::Other(format!("couldn't record ai usage :< {error}")))
    }
}
