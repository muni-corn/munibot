use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{memory::Conversation, tools::ConversationId, types::AiError};

/// One entry in a person's conversation list.
///
/// Deliberately not [`Conversation`]: a sidebar needs the title and whether it
/// is archived, and needs neither the summary nor the persona binding that a
/// running turn cares about.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversationEntry {
    pub id: ConversationId,
    /// `None` until a title has been generated or set, which the interface
    /// renders as a placeholder rather than inventing a name for.
    pub title: Option<String>,
    pub persona_id: String,
    pub last_active_at: DateTime<Utc>,
}

/// Lists, creates, renames, and archives the conversations belonging to one
/// person.
///
/// Separate from [`crate::memory::SessionStore`] on purpose. That trait
/// answers "what was said in this scope"; this one answers "what conversations
/// does this person have", which is a different question with a different
/// index behind it. Folding both into one trait would force every session
/// store to implement ownership semantics, including the in-memory one used by
/// tests and the channel-scoped surfaces that have no owner at all.
#[async_trait]
pub trait ConversationDirectory: Send + Sync {
    /// Every conversation `user_id` owns, most recently active first,
    /// excluding archived ones.
    async fn list_for_user(&self, user_id: u64) -> Result<Vec<ConversationEntry>, AiError>;

    /// Starts a new conversation owned by `user_id`, bound to `persona_id`.
    async fn create_for_user(
        &self,
        user_id: u64,
        persona_id: &str,
    ) -> Result<Conversation, AiError>;

    /// Renames a conversation.
    async fn rename(&self, conversation_id: ConversationId, title: &str) -> Result<(), AiError>;

    /// Hides a conversation from the listing without deleting it, so a
    /// misclick is recoverable.
    async fn archive(&self, conversation_id: ConversationId) -> Result<(), AiError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_an_entry_without_a_title_is_representable() {
        // the interface renders this as a placeholder rather than inventing a name,
        // which is why the field is optional rather than defaulting to a string
        let entry = ConversationEntry {
            id: ConversationId(1),
            title: None,
            persona_id: "companion".to_string(),
            last_active_at: Utc::now(),
        };
        assert!(entry.title.is_none());
    }
}
