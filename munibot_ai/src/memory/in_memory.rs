use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use chrono::Utc;

use crate::{
    memory::{Conversation, ConversationScope, SessionStore},
    tools::ConversationId,
    types::{AiError, History, Message, Role},
};

/// Default cap on messages retained per conversation.
///
/// A crude backstop distinct from summarisation (milestone 2): once exceeded,
/// the oldest messages are simply dropped rather than condensed, just enough to
/// keep an unbounded conversation from growing forever in memory before real
/// compaction exists.
const DEFAULT_MESSAGE_CAP: usize = 200;

struct State {
    next_id: u64,
    by_scope: HashMap<ConversationScope, ConversationId>,
    conversations: HashMap<ConversationId, Conversation>,
    messages: HashMap<ConversationId, Vec<Message>>,
}

impl State {
    fn new() -> Self {
        Self {
            next_id: 1,
            by_scope: HashMap::new(),
            conversations: HashMap::new(),
            messages: HashMap::new(),
        }
    }
}

/// An in-memory [`SessionStore`], sufficient for the whole test suite and the
/// first Discord build.
///
/// Conversation history does not survive a restart - the diesel-backed store in
/// milestone 2 is what makes it durable.
pub struct InMemorySessionStore {
    message_cap: usize,
    state: RwLock<State>,
}

impl InMemorySessionStore {
    /// Builds a store with the default message cap.
    pub fn new() -> Self {
        Self::with_message_cap(DEFAULT_MESSAGE_CAP)
    }

    /// Builds a store with an explicit per-conversation message cap, mainly for
    /// tests that want a small cap to exercise eviction without appending
    /// hundreds of messages.
    pub fn with_message_cap(message_cap: usize) -> Self {
        Self {
            message_cap,
            state: RwLock::new(State::new()),
        }
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Drops messages beyond `cap`, then drops any leading messages that are now
/// orphaned tool results - a `Role::Tool` message whose corresponding `ToolUse`
/// was just evicted would otherwise reference a call the model never actually
/// asked for in what remains.
fn enforce_cap(messages: &mut Vec<Message>, cap: usize) {
    if messages.len() > cap {
        let excess = messages.len() - cap;
        messages.drain(0..excess);
    }
    while messages
        .first()
        .is_some_and(|message| message.role == Role::Tool)
    {
        messages.remove(0);
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load_or_create(
        &self,
        scope: &ConversationScope,
        persona_id: &str,
    ) -> Result<Conversation, AiError> {
        let mut state = self.state.write().unwrap();

        if let Some(id) = state.by_scope.get(scope) {
            return Ok(state.conversations[id].clone());
        }

        let id = ConversationId(state.next_id);
        state.next_id += 1;

        let conversation = Conversation {
            id,
            scope: scope.clone(),
            persona_id: persona_id.to_string(),
            summary: None,
            last_active_at: Utc::now(),
        };

        state.by_scope.insert(scope.clone(), id);
        state.conversations.insert(id, conversation.clone());
        state.messages.insert(id, Vec::new());

        Ok(conversation)
    }

    async fn append(
        &self,
        conversation_id: ConversationId,
        message: Message,
    ) -> Result<(), AiError> {
        let mut state = self.state.write().unwrap();

        let messages = state.messages.entry(conversation_id).or_default();
        messages.push(message);
        enforce_cap(messages, self.message_cap);

        if let Some(conversation) = state.conversations.get_mut(&conversation_id) {
            conversation.last_active_at = Utc::now();
        }

        Ok(())
    }

    async fn history(
        &self,
        conversation_id: ConversationId,
        limit: Option<usize>,
    ) -> Result<History, AiError> {
        let state = self.state.read().unwrap();
        let messages = state
            .messages
            .get(&conversation_id)
            .cloned()
            .unwrap_or_default();

        let windowed = match limit {
            Some(limit) if messages.len() > limit => messages[messages.len() - limit..].to_vec(),
            _ => messages,
        };

        Ok(History::from(windowed))
    }

    async fn set_summary(
        &self,
        conversation_id: ConversationId,
        summary: String,
    ) -> Result<(), AiError> {
        let mut state = self.state.write().unwrap();
        if let Some(conversation) = state.conversations.get_mut(&conversation_id) {
            conversation.summary = Some(summary);
        }
        Ok(())
    }

    async fn clear(&self, conversation_id: ConversationId) -> Result<(), AiError> {
        let mut state = self.state.write().unwrap();
        if let Some(messages) = state.messages.get_mut(&conversation_id) {
            messages.clear();
        }
        if let Some(conversation) = state.conversations.get_mut(&conversation_id) {
            conversation.summary = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Platform;

    fn scope(key: &str) -> ConversationScope {
        ConversationScope::new(Platform::Discord, key)
    }

    #[tokio::test]
    async fn test_load_or_create_is_idempotent_for_the_same_scope() {
        let store = InMemorySessionStore::new();
        let first = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        let second = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        assert_eq!(
            first.id, second.id,
            "the same scope should resolve to the same conversation"
        );
    }

    #[tokio::test]
    async fn test_different_scopes_get_different_conversations() {
        let store = InMemorySessionStore::new();
        let a = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        let b = store
            .load_or_create(&scope("channel-2"), "companion")
            .await
            .unwrap();

        assert_ne!(a.id, b.id);
    }

    #[tokio::test]
    async fn test_existing_conversation_keeps_its_original_persona() {
        // explicit persona switching is the router's job in a later milestone, not
        // something a load call should silently perform
        let store = InMemorySessionStore::new();
        store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        let reloaded = store
            .load_or_create(&scope("channel-1"), "researcher")
            .await
            .unwrap();

        assert_eq!(
            reloaded.persona_id, "companion",
            "the stored persona should not be overwritten"
        );
    }

    #[tokio::test]
    async fn test_append_and_history_round_trip() {
        let store = InMemorySessionStore::new();
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        store
            .append(conversation.id, Message::user("hi"))
            .await
            .unwrap();
        store
            .append(conversation.id, Message::assistant("hello"))
            .await
            .unwrap();

        let history = store.history(conversation.id, None).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_history_returns_messages_oldest_first() {
        let store = InMemorySessionStore::new();
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        store
            .append(conversation.id, Message::user("first"))
            .await
            .unwrap();
        store
            .append(conversation.id, Message::user("second"))
            .await
            .unwrap();

        let history = store.history(conversation.id, None).await.unwrap();
        let texts: Vec<_> = history.iter().map(Message::text).collect();
        assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
    }

    #[tokio::test]
    async fn test_history_limit_returns_the_most_recent_messages() {
        let store = InMemorySessionStore::new();
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        for text in ["one", "two", "three"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }

        let history = store.history(conversation.id, Some(2)).await.unwrap();
        let texts: Vec<_> = history.iter().map(Message::text).collect();
        assert_eq!(
            texts,
            vec!["two".to_string(), "three".to_string()],
            "a limit should keep the most recent messages, still oldest-first"
        );
    }

    #[tokio::test]
    async fn test_history_for_an_unknown_conversation_is_empty() {
        let store = InMemorySessionStore::new();
        let history = store.history(ConversationId(999), None).await.unwrap();
        assert!(history.is_empty());
    }

    #[tokio::test]
    async fn test_set_summary_is_visible_on_reload() {
        let store = InMemorySessionStore::new();
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        store
            .set_summary(conversation.id, "a condensed summary".to_string())
            .await
            .unwrap();

        let reloaded = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        assert_eq!(reloaded.summary.as_deref(), Some("a condensed summary"));
    }

    #[tokio::test]
    async fn test_clear_empties_history_but_keeps_the_conversation() {
        let store = InMemorySessionStore::new();
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        store
            .append(conversation.id, Message::user("hi"))
            .await
            .unwrap();
        store
            .set_summary(conversation.id, "summary".to_string())
            .await
            .unwrap();

        store.clear(conversation.id).await.unwrap();

        let history = store.history(conversation.id, None).await.unwrap();
        assert!(history.is_empty(), "clear should empty the history");

        let reloaded = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();
        assert_eq!(
            reloaded.id, conversation.id,
            "clear must not delete the scope-to-id mapping"
        );
        assert_eq!(reloaded.summary, None, "clear should also drop the summary");
    }

    #[tokio::test]
    async fn test_message_cap_evicts_the_oldest_messages() {
        let store = InMemorySessionStore::with_message_cap(2);
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        for text in ["one", "two", "three"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }

        let history = store.history(conversation.id, None).await.unwrap();
        let texts: Vec<_> = history.iter().map(Message::text).collect();
        assert_eq!(
            texts,
            vec!["two".to_string(), "three".to_string()],
            "the cap should evict from the front, keeping the most recent messages"
        );
    }

    #[tokio::test]
    async fn test_message_cap_eviction_drops_an_orphaned_leading_tool_result() {
        use serde_json::json;

        use crate::types::ContentBlock;

        // cap of 1: appending the tool call brings the count to 2, evicting the user
        // message that came before it; appending the tool result then brings it
        // to 2 again, evicting the tool call itself and leaving the tool result
        // as what would be an orphaned leading message - exactly the case the
        // cascade-drop exists for
        let store = InMemorySessionStore::with_message_cap(1);
        let conversation = store
            .load_or_create(&scope("channel-1"), "companion")
            .await
            .unwrap();

        store
            .append(conversation.id, Message::user("first"))
            .await
            .unwrap();
        store
            .append(
                conversation.id,
                Message::new(Role::Assistant, vec![ContentBlock::tool_use(
                    "c1",
                    "current_time",
                    json!({}),
                )]),
            )
            .await
            .unwrap();
        store
            .append(
                conversation.id,
                Message::tool_results(vec![ContentBlock::tool_result("c1", "12:00")]),
            )
            .await
            .unwrap();

        let history = store.history(conversation.id, None).await.unwrap();
        assert!(
            history.is_empty(),
            "the orphaned tool result must be cascade-dropped, leaving nothing rather than a \
             broken leading message: {history:?}"
        );

        // a following ordinary message should still append normally afterward
        store
            .append(conversation.id, Message::user("later"))
            .await
            .unwrap();
        let history = store.history(conversation.id, None).await.unwrap();
        assert_eq!(
            history.iter().next().map(Message::text),
            Some("later".to_string())
        );
    }
}
