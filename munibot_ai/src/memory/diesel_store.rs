use async_trait::async_trait;
use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, models::AiConversation, operations::ai};

use crate::{
    memory::{Conversation, ConversationScope, SessionStore},
    tools::{ConversationId, Platform},
    types::{AiError, ContentBlock, History, Message, Role, rough_token_estimate},
};

/// A [`SessionStore`] backed by MySQL through `diesel-async`.
///
/// The production counterpart to [`crate::memory::InMemorySessionStore`],
/// which remains the store every unit test uses. No feature gate: this crate
/// has depended on `munibot_core` unconditionally since the persona
/// configuration landed.
pub struct DieselSessionStore {
    pool: DbPool,
    /// Set on a conversation when it is created for a signed-in person, so the
    /// web sidebar can list it. `None` for channel-scoped conversations, which
    /// belong to a place rather than a person.
    owner_user_id: Option<i64>,
}

impl DieselSessionStore {
    /// Builds a store for channel-scoped conversations, which have no owner.
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            owner_user_id: None,
        }
    }

    /// Builds a store whose newly created conversations belong to `user_id`.
    ///
    /// Ownership is fixed per store rather than passed per call because
    /// [`SessionStore::load_or_create`] has no argument for it, and widening
    /// that trait would force every implementation to care about a concept
    /// only the web surface has.
    pub fn owned_by(pool: DbPool, user_id: i64) -> Self {
        Self {
            pool,
            owner_user_id: Some(user_id),
        }
    }
}

/// Database failures surface as [`AiError::Other`] rather than a dedicated
/// variant: nothing above this layer can act differently on a connection error
/// than on any other unexpected failure, and adding a variant every caller
/// would immediately collapse back together earns nothing.
fn db_error(error: impl std::fmt::Display) -> AiError {
    AiError::Other(format!("the database had trouble :< {error}"))
}

/// Converts a stored row into the domain type, defaulting a scope whose
/// platform string is unrecognised to [`Platform::Web`].
///
/// An unknown platform means a row written by a newer version, which is worth
/// surfacing but not worth failing a conversation over.
fn to_conversation(row: AiConversation) -> Conversation {
    let platform = Platform::from_key(&row.platform).unwrap_or_else(|| {
        tracing::warn!(
            platform = %row.platform,
            conversation_id = row.id,
            "unrecognised platform on a stored conversation; treating it as web"
        );
        Platform::Web
    });

    Conversation {
        id: ConversationId(row.id as u64),
        scope: ConversationScope::new(platform, row.scope_key),
        persona_id: row.persona_id,
        summary: row.summary,
        last_active_at: DateTime::<Utc>::from_naive_utc_and_offset(row.last_active_at, Utc),
    }
}

/// Estimates one message's token cost the same way the rest of the crate does.
fn message_tokens(message: &Message) -> i32 {
    let estimate = History::from(vec![message.clone()]).token_estimate(rough_token_estimate);
    i32::try_from(estimate).unwrap_or(i32::MAX)
}

#[async_trait]
impl SessionStore for DieselSessionStore {
    async fn load_or_create(
        &self,
        scope: &ConversationScope,
        persona_id: &str,
    ) -> Result<Conversation, AiError> {
        let row = ai::get_or_create_conversation(
            &self.pool,
            scope.platform.as_key(),
            &scope.scope_key,
            persona_id,
            self.owner_user_id,
        )
        .await
        .map_err(db_error)?;

        Ok(to_conversation(row))
    }

    async fn append(
        &self,
        conversation_id: ConversationId,
        message: Message,
    ) -> Result<(), AiError> {
        let content = serde_json::to_string(&message.content).map_err(|error| {
            AiError::Other(format!("couldn't encode a message to store it :< {error}"))
        })?;

        ai::append_message(
            &self.pool,
            conversation_id.0 as i64,
            message.role.as_key(),
            &content,
            message_tokens(&message),
        )
        .await
        .map_err(db_error)?;

        Ok(())
    }

    async fn history(
        &self,
        conversation_id: ConversationId,
        limit: Option<usize>,
    ) -> Result<History, AiError> {
        let limit = limit.map(|n| i64::try_from(n).unwrap_or(i64::MAX));
        let rows = ai::get_messages(&self.pool, conversation_id.0 as i64, limit)
            .await
            .map_err(db_error)?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            // a row that cannot be decoded is skipped rather than failing the whole
            // turn: one malformed message should not make a conversation permanently
            // unusable, and the alternative is a user who can never talk to munibot
            // again in that conversation
            let Some(role) = Role::from_key(&row.role) else {
                tracing::warn!(role = %row.role, seq = row.seq, "skipping a message with an unrecognised role");
                continue;
            };
            match serde_json::from_str::<Vec<ContentBlock>>(&row.content) {
                Ok(content) => messages.push(Message::new(role, content)),
                Err(error) => {
                    tracing::warn!(%error, seq = row.seq, "skipping a message that failed to decode");
                }
            }
        }

        Ok(History::from(messages))
    }

    async fn set_summary(
        &self,
        conversation_id: ConversationId,
        summary: String,
    ) -> Result<(), AiError> {
        let tokens = i32::try_from(rough_token_estimate(&summary)).unwrap_or(i32::MAX);
        ai::set_conversation_summary(&self.pool, conversation_id.0 as i64, Some(&summary), tokens)
            .await
            .map_err(db_error)
    }

    async fn clear(&self, conversation_id: ConversationId) -> Result<(), AiError> {
        ai::clear_conversation(&self.pool, conversation_id.0 as i64)
            .await
            .map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_an_unrecognised_platform_falls_back_to_web_rather_than_failing() {
        let row = AiConversation {
            id: 1,
            platform: "matrix".to_string(),
            scope_key: "room-1".to_string(),
            persona_id: "companion".to_string(),
            owner_user_id: None,
            title: None,
            summary: None,
            summary_tokens: 0,
            archived_at: None,
            created_at: Utc::now().naive_utc(),
            last_active_at: Utc::now().naive_utc(),
        };

        let conversation = to_conversation(row);
        assert_eq!(conversation.scope.platform, Platform::Web);
        assert_eq!(conversation.id, ConversationId(1));
    }

    #[test]
    fn test_a_known_platform_round_trips_through_the_row() {
        let row = AiConversation {
            id: 7,
            platform: Platform::Discord.as_key().to_string(),
            scope_key: "channel-1".to_string(),
            persona_id: "companion".to_string(),
            owner_user_id: Some(3),
            title: Some("about cats".to_string()),
            summary: Some("we talked about cats".to_string()),
            summary_tokens: 5,
            archived_at: None,
            created_at: Utc::now().naive_utc(),
            last_active_at: Utc::now().naive_utc(),
        };

        let conversation = to_conversation(row);
        assert_eq!(conversation.scope.platform, Platform::Discord);
        assert_eq!(conversation.scope.scope_key, "channel-1");
        assert_eq!(
            conversation.summary.as_deref(),
            Some("we talked about cats")
        );
    }

    #[test]
    fn test_message_tokens_are_never_negative_or_overflowing() {
        let message = Message::user("hello there");
        assert!(message_tokens(&message) > 0);
    }
}
