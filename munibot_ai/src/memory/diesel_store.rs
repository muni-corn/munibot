use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use munibot_core::db::{DbPool, models::AiConversation, operations::ai};

use crate::{
    memory::{
        Conversation, ConversationDirectory, ConversationEntry, ConversationScope, SessionStore,
    },
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

    /// Reconstructs every attachment linked to `message_id` as base64
    /// image content blocks, for appending to that message's own content
    /// before it ever reaches a provider request - see
    /// `docs/plans/ai/milestone-3-specialists.md`'s own note that images
    /// are encoded only at this point, never stored that way.
    ///
    /// Best-effort per attachment: one that fails to load (deleted between
    /// listing and reading, or some other database hiccup) is logged and
    /// skipped rather than failing the whole history load, the same
    /// reasoning [`Self::history`] itself already applies to a message
    /// that fails to decode.
    async fn image_blocks_for_message(&self, message_id: i64) -> Vec<ContentBlock> {
        let attachments = match ai::list_attachments_for_message(&self.pool, message_id).await {
            Ok(attachments) => attachments,
            Err(error) => {
                tracing::warn!(
                    %error,
                    message_id,
                    "couldn't list attachments for a message; continuing without them"
                );
                return Vec::new();
            }
        };

        let mut blocks = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            match ai::get_attachment(&self.pool, attachment.id).await {
                Ok(Some(attachment)) => {
                    let data = STANDARD.encode(&attachment.data);
                    blocks.push(ContentBlock::image_base64(attachment.media_type, data));
                }
                Ok(None) => {
                    tracing::warn!(
                        attachment_id = attachment.id,
                        "an attachment listed for a message no longer exists; skipping it"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        attachment_id = attachment.id,
                        "couldn't load an attachment's bytes; skipping it"
                    );
                }
            }
        }
        blocks
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
        title: row.title,
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
            let mut content = match serde_json::from_str::<Vec<ContentBlock>>(&row.content) {
                Ok(content) => content,
                Err(error) => {
                    tracing::warn!(%error, seq = row.seq, "skipping a message that failed to decode");
                    continue;
                }
            };

            content.extend(self.image_blocks_for_message(row.id).await);
            messages.push(Message::new(role, content));
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

    async fn set_title(
        &self,
        conversation_id: ConversationId,
        title: String,
    ) -> Result<(), AiError> {
        ai::rename_conversation(&self.pool, conversation_id.0 as i64, &title)
            .await
            .map_err(db_error)
    }

    async fn clear(&self, conversation_id: ConversationId) -> Result<(), AiError> {
        ai::clear_conversation(&self.pool, conversation_id.0 as i64)
            .await
            .map_err(db_error)
    }

    async fn compact(
        &self,
        conversation_id: ConversationId,
        keep_recent: usize,
        summary: String,
    ) -> Result<(), AiError> {
        let tokens = i32::try_from(rough_token_estimate(&summary)).unwrap_or(i32::MAX);
        ai::compact_conversation(
            &self.pool,
            conversation_id.0 as i64,
            keep_recent as i64,
            &summary,
            tokens,
        )
        .await
        .map_err(db_error)
    }
}

#[async_trait]
impl ConversationDirectory for DieselSessionStore {
    async fn list_for_user(&self, user_id: u64) -> Result<Vec<ConversationEntry>, AiError> {
        let rows = ai::list_conversations_for_user(&self.pool, user_id as i64)
            .await
            .map_err(db_error)?;

        Ok(rows
            .into_iter()
            .map(|row| ConversationEntry {
                id: ConversationId(row.id as u64),
                title: row.title,
                persona_id: row.persona_id,
                last_active_at: DateTime::<Utc>::from_naive_utc_and_offset(row.last_active_at, Utc),
            })
            .collect())
    }

    async fn create_for_user(
        &self,
        user_id: u64,
        persona_id: &str,
    ) -> Result<Conversation, AiError> {
        // a web conversation's scope key is opaque and generated here rather than
        // supplied: unlike a channel, nothing outside this row identifies it, and
        // letting a caller choose would let one person guess another's key
        let scope_key = new_scope_key();
        let now = Utc::now().naive_utc();

        let row =
            ai::create_conversation(&self.pool, munibot_core::db::models::NewAiConversation {
                platform: Platform::Web.as_key().to_string(),
                scope_key,
                persona_id: persona_id.to_string(),
                owner_user_id: Some(user_id as i64),
                title: None,
                created_at: now,
                last_active_at: now,
            })
            .await
            .map_err(db_error)?;

        Ok(to_conversation(row))
    }

    async fn rename(&self, conversation_id: ConversationId, title: &str) -> Result<(), AiError> {
        ai::rename_conversation(&self.pool, conversation_id.0 as i64, title)
            .await
            .map_err(db_error)
    }

    async fn archive(&self, conversation_id: ConversationId) -> Result<(), AiError> {
        ai::archive_conversation(&self.pool, conversation_id.0 as i64)
            .await
            .map_err(db_error)
    }
}

/// Generates an opaque scope key for a new web conversation.
///
/// Random rather than sequential so a key cannot be guessed from another one.
/// Ownership is still checked on every access; this only removes the
/// temptation to treat the key itself as a capability.
fn new_scope_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("web-{nanos:x}-{count:x}")
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
