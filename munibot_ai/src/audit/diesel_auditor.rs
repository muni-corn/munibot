use async_trait::async_trait;
use munibot_core::db::{DbPool, models::NewAiToolCall, operations::ai};

use crate::audit::{ToolAuditor, ToolCallRecord};

/// A [`ToolAuditor`] backed by MySQL through `diesel-async`.
pub struct DieselToolAuditor {
    pool: DbPool,
}

impl DieselToolAuditor {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ToolAuditor for DieselToolAuditor {
    async fn record(&self, record: ToolCallRecord) {
        let row = NewAiToolCall {
            conversation_id: record.conversation_id.map(|id| id.0 as i64),
            tool_name: record.tool_name,
            input: Some(record.input),
            output: Some(record.output),
            duration_ms: i64::try_from(record.duration.as_millis()).unwrap_or(i64::MAX),
            status: record.status.as_key().to_string(),
            created_at: chrono::Utc::now().naive_utc(),
        };

        if let Err(error) = ai::record_tool_call(&self.pool, row).await {
            tracing::warn!(%error, "couldn't record a tool call audit row");
        }
    }
}
