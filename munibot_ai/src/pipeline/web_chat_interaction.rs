//! `WebChatAdapter`: the primary [`InteractionAdapter`] -- a pipeline
//! question arrives as a message from munibot in the conversation you are
//! already having with him, and your reply resumes the run.
//!
//! Not a Discord thread, as originally planned: the web chat is the
//! primary surface, and routing a question through the companion means it
//! inherits streaming, the tool activity strip, and the delegation
//! display for free, rather than a second notification surface with none
//! of that. The fallback when nobody is signed in to answer here remains
//! [`crate::pipeline::GitHubCommentAdapter`] -- whichever caller wires up
//! a real pipeline run decides which of the two to construct, based on
//! whether a maintainer is actually signed in.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::{
    memory::SessionStore,
    pipeline::{
        InteractionAdapter, InteractionError, InteractionRequest, InteractionResponse, PipelineId,
    },
    tools::ConversationId,
    types::Message,
};

/// Tracks pipelines waiting on a reply in a specific web chat
/// conversation.
///
/// The web chat itself is an ordinary request/response surface -- a
/// browser posts a message, gets a streamed answer back -- with no
/// existing push mechanism for "wait for whatever the user sends next".
/// This is that mechanism: `register` hands back a receiver a pending
/// `request_input` call awaits, and whatever handles an incoming chat
/// message is expected to call `resolve_reply` for that conversation
/// *before* treating the message as an ordinary turn, so a reply meant to
/// resume a paused pipeline is never also sent to whichever persona the
/// conversation would otherwise route to.
#[derive(Default)]
pub struct PendingReplyRegistry {
    pending: Mutex<HashMap<ConversationId, oneshot::Sender<String>>>,
}

impl PendingReplyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a wait for the next reply in `conversation_id`.
    ///
    /// Replaces any previous, still-pending registration for the same
    /// conversation rather than erroring -- a pipeline only ever waits on
    /// one question at a time, so a second registration can only mean the
    /// first was abandoned (its own receiver dropped).
    pub fn register(&self, conversation_id: ConversationId) -> oneshot::Receiver<String> {
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .expect("registry lock poisoned")
            .insert(conversation_id, sender);
        receiver
    }

    /// Resolves whatever is waiting on `conversation_id`, if anything.
    /// Returns whether a pending wait was actually found -- the real
    /// chat message handler uses this to decide whether an incoming
    /// message was a pipeline's own answer or an ordinary chat turn.
    pub fn resolve_reply(&self, conversation_id: ConversationId, reply: impl Into<String>) -> bool {
        let sender = self
            .pending
            .lock()
            .expect("registry lock poisoned")
            .remove(&conversation_id);

        match sender {
            Some(sender) => sender.send(reply.into()).is_ok(),
            None => false,
        }
    }
}

/// Delivers a pipeline's question as a message from munibot in an
/// existing web chat conversation, and resumes once
/// [`PendingReplyRegistry::resolve_reply`] is called for it.
pub struct WebChatAdapter {
    conversation_id: ConversationId,
    sessions: std::sync::Arc<dyn SessionStore>,
    replies: std::sync::Arc<PendingReplyRegistry>,
}

impl WebChatAdapter {
    pub fn new(
        conversation_id: ConversationId,
        sessions: std::sync::Arc<dyn SessionStore>,
        replies: std::sync::Arc<PendingReplyRegistry>,
    ) -> Self {
        Self {
            conversation_id,
            sessions,
            replies,
        }
    }
}

#[async_trait]
impl InteractionAdapter for WebChatAdapter {
    async fn request_input(
        &self,
        pipeline_id: PipelineId,
        request: &InteractionRequest,
    ) -> Result<InteractionResponse, InteractionError> {
        // registered before the message is even sent -- a reply arriving
        // the instant it's posted must never race the registration itself
        let receiver = self.replies.register(self.conversation_id);

        self.sessions
            .append(
                self.conversation_id,
                Message::assistant(request.prompt.clone()),
            )
            .await
            .map_err(|error| InteractionError::Delivery(pipeline_id, error.to_string()))?;

        receiver
            .await
            .map_err(|_| {
                InteractionError::Delivery(
                    pipeline_id,
                    "the reply channel closed before an answer arrived".to_string(),
                )
            })
            .map(InteractionResponse::new)
    }

    async fn notify(&self, pipeline_id: PipelineId, message: &str) -> Result<(), InteractionError> {
        self.sessions
            .append(self.conversation_id, Message::assistant(message))
            .await
            .map_err(|error| InteractionError::Notification(pipeline_id, error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        memory::{ConversationScope, InMemorySessionStore},
        tools::Platform,
        types::ContentBlock,
    };

    fn contains_text(history: &crate::types::History, text: &str) -> bool {
        history.messages().iter().any(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::Text { text: t } if t == text))
        })
    }

    fn conversation_id() -> ConversationId {
        ConversationId(1)
    }

    async fn session_store_with_a_conversation() -> Arc<InMemorySessionStore> {
        let store = Arc::new(InMemorySessionStore::new());
        let scope = ConversationScope {
            platform: Platform::Web,
            scope_key: "test".to_string(),
        };
        let conversation = store.load_or_create(&scope, "companion").await.unwrap();
        assert_eq!(conversation.id, conversation_id());
        store
    }

    #[test]
    fn test_register_then_resolve_delivers_the_reply() {
        let registry = PendingReplyRegistry::new();
        let mut receiver = registry.register(conversation_id());

        assert!(registry.resolve_reply(conversation_id(), "postgres"));
        assert_eq!(receiver.try_recv().unwrap(), "postgres");
    }

    #[test]
    fn test_resolve_reply_returns_false_when_nothing_is_waiting() {
        let registry = PendingReplyRegistry::new();
        assert!(!registry.resolve_reply(conversation_id(), "unsolicited"));
    }

    #[test]
    fn test_resolve_reply_only_delivers_to_the_matching_conversation() {
        let registry = PendingReplyRegistry::new();
        let mut receiver = registry.register(ConversationId(1));

        assert!(!registry.resolve_reply(ConversationId(2), "wrong conversation"));
        assert!(
            receiver.try_recv().is_err(),
            "the receiver should not have resolved"
        );
    }

    #[tokio::test]
    async fn test_request_input_posts_the_question_into_the_conversation() {
        let store = session_store_with_a_conversation().await;
        let replies = Arc::new(PendingReplyRegistry::new());
        let adapter = WebChatAdapter::new(conversation_id(), store.clone(), replies.clone());

        let request = InteractionRequest {
            prompt: "which database should this use?".to_string(),
        };

        tokio::spawn({
            let replies = replies.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                replies.resolve_reply(conversation_id(), "postgres");
            }
        });

        let response = adapter
            .request_input(PipelineId(1), &request)
            .await
            .unwrap();
        assert_eq!(response.response, "postgres");

        let history = store.history(conversation_id(), None).await.unwrap();
        assert!(
            contains_text(&history, &request.prompt),
            "the question itself should have been appended to the conversation"
        );
    }

    #[tokio::test]
    async fn test_notify_appends_a_message_with_no_reply_expected() {
        let store = session_store_with_a_conversation().await;
        let replies = Arc::new(PendingReplyRegistry::new());
        let adapter = WebChatAdapter::new(conversation_id(), store.clone(), replies);

        adapter
            .notify(PipelineId(1), "opened a pull request")
            .await
            .unwrap();

        let history = store.history(conversation_id(), None).await.unwrap();
        assert!(contains_text(&history, "opened a pull request"));
    }
}
