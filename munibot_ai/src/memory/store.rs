use async_trait::async_trait;

use crate::{
    memory::{Conversation, ConversationScope},
    tools::ConversationId,
    types::{AiError, History, Message},
};

/// Stores conversations and their message history.
///
/// The one thing every implementation must agree on: `load_or_create` is
/// idempotent per scope, `append` never reorders or drops a message, and
/// `history` returns messages oldest first, exactly as
/// [`crate::types::History`] expects them fed back to a provider.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Loads the conversation for a scope, creating one bound to `persona_id`
    /// if none exists yet.
    ///
    /// An existing conversation's `persona_id` is returned as stored, even if
    /// it differs from what is passed here - explicit persona switching is
    /// the router's job in a later milestone, not something a load call
    /// should silently perform.
    async fn load_or_create(
        &self,
        scope: &ConversationScope,
        persona_id: &str,
    ) -> Result<Conversation, AiError>;

    /// Appends one message to a conversation's history.
    async fn append(
        &self,
        conversation_id: ConversationId,
        message: Message,
    ) -> Result<(), AiError>;

    /// Returns a conversation's history, oldest first.
    ///
    /// `limit` caps how many of the most recent messages come back; `None`
    /// returns everything stored. This is a message count, not a token
    /// budget - the token-aware walk lives in
    /// [`crate::memory::assemble_context`], built on top of this.
    async fn history(
        &self,
        conversation_id: ConversationId,
        limit: Option<usize>,
    ) -> Result<History, AiError>;

    /// Sets or replaces a conversation's summary, for once compaction lands in
    /// milestone 2.
    async fn set_summary(
        &self,
        conversation_id: ConversationId,
        summary: String,
    ) -> Result<(), AiError>;

    /// Clears a conversation's history and summary, without deleting the
    /// conversation row (or its scope-to-id mapping) itself - a fresh
    /// `/reset` should still resolve to the same conversation, just an
    /// empty one.
    async fn clear(&self, conversation_id: ConversationId) -> Result<(), AiError>;

    /// Deletes every message older than the most recent `keep_recent`, and
    /// sets `summary` to describe what was removed.
    ///
    /// A conversation with `keep_recent` messages or fewer is left completely
    /// untouched - there is nothing to summarise, and calling this on a short
    /// conversation must never overwrite an existing summary with one that
    /// describes nothing. This is the primitive
    /// [`crate::memory::compact_if_needed`] builds on; callers should
    /// generally reach for that instead of this directly, since it is what
    /// decides *whether* compaction is worth doing at all.
    async fn compact(
        &self,
        conversation_id: ConversationId,
        keep_recent: usize,
        summary: String,
    ) -> Result<(), AiError>;
}
