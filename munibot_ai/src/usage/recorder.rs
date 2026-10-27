use async_trait::async_trait;

use crate::{
    tools::ConversationId,
    types::{AiError, Cost, Usage},
};

/// What one turn cost, whether or not it succeeded.
///
/// `conversation_id`, `user_id`, and `guild_id` are each independently
/// optional at the type level, even though [`crate::service::Ai`] currently
/// always has a conversation and a user by the time it builds one of these:
/// a direct message has no guild, and a future caller outside the chat
/// surfaces (the pipeline, say) may have no conversation at all.
#[derive(Clone, Debug)]
pub struct UsageRecord {
    pub conversation_id: Option<ConversationId>,
    pub user_id: Option<u64>,
    pub guild_id: Option<u64>,
    pub provider: String,
    pub model: String,
    pub persona_id: String,
    pub usage: Usage,
    pub cost: Cost,
    pub iterations: usize,
    /// `false` for a turn that ended in an error. Recorded either way: a
    /// turn that failed on its ninth iteration still spent the first eight,
    /// and a usage table that only records successes understates spend
    /// exactly when something is going wrong.
    pub succeeded: bool,
    /// The pipeline run this turn belongs to, if any - an opaque foreign
    /// id rather than `crate::pipeline::PipelineId`: `ai::usage` sits
    /// below `ai::pipeline` in the dependency graph (see
    /// `crate::project::Project::pipeline_id`'s own doc comment for the
    /// same reasoning), so it cannot name that module's types. `None`
    /// for every plain chat turn, which has no pipeline behind it at all.
    pub pipeline_id: Option<i64>,
}

/// Records what a turn cost.
///
/// A recorder failing to write is never allowed to fail the turn itself -
/// see how [`crate::service::Ai`] calls this, best-effort, after the fact.
#[async_trait]
pub trait UsageRecorder: Send + Sync {
    async fn record(&self, record: UsageRecord) -> Result<(), AiError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> UsageRecord {
        UsageRecord {
            conversation_id: Some(ConversationId(1)),
            user_id: Some(7),
            guild_id: None,
            provider: "anthropic".to_string(),
            model: "claude-opus-5".to_string(),
            persona_id: "companion".to_string(),
            usage: Usage::new(10, 20),
            cost: Cost::from_micros(500),
            iterations: 2,
            succeeded: true,
            pipeline_id: None,
        }
    }

    #[test]
    fn test_a_usage_record_is_constructible_with_no_guild() {
        // the common case: a direct message or a web conversation has no guild
        // at all
        let record = record();
        assert_eq!(record.guild_id, None);
        assert!(record.succeeded);
    }
}
