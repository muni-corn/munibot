use std::sync::Arc;

use crate::{
    memory::{Conversation, SessionStore},
    provider::Provider,
    types::{AiError, CompletionRequest, ContentBlock, History, Message, ModelRef, Role},
};

/// The model and system prompt used to condense old conversation history into
/// prose, for [`compact_if_needed`].
///
/// Deliberately **not** a full [`crate::persona::Persona`], even though the
/// plan this was built from originally called for one. Compaction is a single
/// one-shot completion with no tools, no per-turn budget, and no handoff -
/// carrying the whole persona type here would let a misconfigured compaction
/// role smuggle in a tool allowlist that means nothing in a plain completion
/// call, and would pull this module into a dependency on `crate::persona`
/// that the crate's own layering forbids (`persona` depends on `memory`, not
/// the other way around).
#[derive(Clone, Debug)]
pub struct CompactionPersona {
    pub model: ModelRef,
    pub system_prompt: String,
}

impl CompactionPersona {
    /// Builds a compaction persona using the embedded default prompt, which
    /// takes no template variables and so needs no rendering step.
    pub fn embedded(model: ModelRef) -> Self {
        Self {
            model,
            system_prompt: include_str!("../../prompts/compaction.md").to_string(),
        }
    }
}

/// How aggressively [`compact_if_needed`] triggers, and how much of a
/// conversation it always leaves untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactionSettings {
    /// Compaction runs once a conversation's full stored history exceeds this
    /// many tokens.
    pub threshold_tokens: usize,
    /// The number of most recent messages that are never summarised away,
    /// regardless of how far over the threshold a conversation is.
    pub keep_recent_messages: usize,
}

impl Default for CompactionSettings {
    /// Comfortably above a typical per-turn context budget: compaction should
    /// trigger well before a conversation's own context window is actually
    /// squeezed, not the instant it is.
    fn default() -> Self {
        Self {
            threshold_tokens: 12_000,
            keep_recent_messages: 20,
        }
    }
}

/// Condenses old conversation history into an updated prose summary.
pub struct Summariser {
    provider: Arc<dyn Provider>,
    persona: CompactionPersona,
}

impl Summariser {
    pub fn new(provider: Arc<dyn Provider>, persona: CompactionPersona) -> Self {
        Self { provider, persona }
    }

    /// Asks the model to fold `existing_summary` and `to_summarise` into one
    /// updated summary.
    async fn summarise(
        &self,
        existing_summary: Option<&str>,
        to_summarise: &[Message],
    ) -> Result<String, AiError> {
        let mut user_text = String::new();
        if let Some(existing) = existing_summary {
            user_text.push_str("Existing summary of earlier parts of this conversation:\n");
            user_text.push_str(existing);
            user_text.push_str("\n\n");
        }
        user_text.push_str("Messages to fold into the summary:\n\n");
        user_text.push_str(&render_transcript(to_summarise));

        let request = CompletionRequest::new(
            self.persona.model.clone(),
            History::from(vec![Message::user(user_text)]),
        )
        .with_system(self.persona.system_prompt.clone());

        let response = self.provider.complete(request).await?;
        let summary = response.text();

        if summary.trim().is_empty() {
            return Err(AiError::Other(
                "the compaction model returned an empty summary :<".to_string(),
            ));
        }

        Ok(summary)
    }
}

/// Compacts `conversation` when its full stored history exceeds
/// `settings.threshold_tokens`, keeping the most recent
/// `settings.keep_recent_messages` intact and condensing everything before
/// them into an updated summary.
///
/// A no-op below the threshold, so a short conversation never pays for a
/// summarisation call - this is what makes compaction triggered by actual
/// conversation size rather than a timer. Also a no-op when there are not
/// more messages than `keep_recent_messages` to begin with, on the same
/// principle [`crate::memory::assemble_context`] uses for its own token walk:
/// the most recent messages are never sacrificed, even if that means staying
/// over the threshold for one more turn.
///
/// Returns the new summary when compaction ran, so a caller can update its
/// own in-memory copy of `conversation` without a second read.
pub async fn compact_if_needed(
    summariser: &Summariser,
    store: &dyn SessionStore,
    conversation: &Conversation,
    settings: CompactionSettings,
    counter: impl Fn(&str) -> usize,
) -> Result<Option<String>, AiError> {
    let history = store.history(conversation.id, None).await?;
    if history.token_estimate(&counter) <= settings.threshold_tokens {
        return Ok(None);
    }

    let messages = history.into_messages();
    if messages.len() <= settings.keep_recent_messages {
        return Ok(None);
    }

    let to_summarise = &messages[..messages.len() - settings.keep_recent_messages];
    let summary = summariser
        .summarise(conversation.summary.as_deref(), to_summarise)
        .await?;

    store
        .compact(
            conversation.id,
            settings.keep_recent_messages,
            summary.clone(),
        )
        .await?;

    Ok(Some(summary))
}

/// Renders messages as a plain-text transcript for the compaction prompt.
///
/// Not the same rendering [`Message::text`] gives: that ignores tool traffic
/// entirely, which would silently drop exactly the kind of thing the
/// compaction prompt is told to preserve ("the names of tools that were used
/// and what they found").
fn render_transcript(messages: &[Message]) -> String {
    messages
        .iter()
        .map(render_message)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_message(message: &Message) -> String {
    let label = match message.role {
        Role::System => "System",
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::Tool => "Tool result",
    };

    let parts: Vec<String> = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::ToolUse { name, .. } => Some(format!("(called {name})")),
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => None,
        })
        .collect();

    format!("{label}: {}", parts.join(" "))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{memory::InMemorySessionStore, provider::MockProvider, tools::Platform};

    fn scope() -> crate::memory::ConversationScope {
        crate::memory::ConversationScope::new(Platform::Web, "channel-1")
    }

    fn settings(threshold_tokens: usize, keep_recent_messages: usize) -> CompactionSettings {
        CompactionSettings {
            threshold_tokens,
            keep_recent_messages,
        }
    }

    fn summariser(provider: Arc<MockProvider>) -> Summariser {
        Summariser::new(
            provider,
            CompactionPersona::embedded(ModelRef::new("anthropic", "claude-haiku-4")),
        )
    }

    #[test]
    fn test_render_transcript_includes_tool_call_names_and_results() {
        let messages = vec![
            Message::new(Role::Assistant, vec![ContentBlock::tool_use(
                "c1",
                "web_search",
                json!({"query": "cats"}),
            )]),
            Message::tool_results(vec![ContentBlock::tool_result("c1", "found some cats")]),
        ];

        let transcript = render_transcript(&messages);
        assert!(transcript.contains("web_search"));
        assert!(transcript.contains("found some cats"));
    }

    #[test]
    fn test_render_transcript_labels_each_role() {
        let messages = vec![Message::user("hi"), Message::assistant("hello")];
        let transcript = render_transcript(&messages);
        assert!(transcript.contains("User: hi"));
        assert!(transcript.contains("Assistant: hello"));
    }

    #[tokio::test]
    async fn test_compact_if_needed_is_a_noop_below_the_threshold() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        store
            .append(conversation.id, Message::user("hi"))
            .await
            .unwrap();

        let provider = Arc::new(MockProvider::new());
        let result = compact_if_needed(
            &summariser(provider.clone()),
            &store,
            &conversation,
            settings(1_000_000, 1),
            crate::types::rough_token_estimate,
        )
        .await
        .expect("should succeed");

        assert_eq!(result, None);
        assert_eq!(
            provider.request_count(),
            0,
            "a conversation well under the threshold should never call the model"
        );
    }

    #[tokio::test]
    async fn test_compact_if_needed_is_a_noop_when_there_is_nothing_to_cut() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        store
            .append(
                conversation.id,
                Message::user("a message long enough to be over any tiny threshold"),
            )
            .await
            .unwrap();

        let provider = Arc::new(MockProvider::new());
        // threshold_tokens is 0, so the token check alone would trigger compaction,
        // but keep_recent_messages (5) exceeds the actual message count (1)
        let result = compact_if_needed(
            &summariser(provider.clone()),
            &store,
            &conversation,
            settings(0, 5),
            crate::types::rough_token_estimate,
        )
        .await
        .expect("should succeed");

        assert_eq!(result, None);
        assert_eq!(
            provider.request_count(),
            0,
            "the most recent messages are never sacrificed, even over threshold"
        );
    }

    #[tokio::test]
    async fn test_compact_if_needed_summarises_and_compacts_when_over_threshold() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        for text in ["one", "two", "three", "four"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }

        let provider = Arc::new(MockProvider::new().respond_text("condensed summary"));
        let result = compact_if_needed(
            &summariser(provider.clone()),
            &store,
            &conversation,
            settings(0, 2),
            crate::types::rough_token_estimate,
        )
        .await
        .expect("should succeed");

        assert_eq!(result, Some("condensed summary".to_string()));
        assert_eq!(provider.request_count(), 1);

        let history = store.history(conversation.id, None).await.unwrap();
        let texts: Vec<String> = history.iter().map(Message::text).collect();
        assert_eq!(
            texts,
            vec!["three".to_string(), "four".to_string()],
            "the store should reflect the compaction, not just the returned summary"
        );
    }

    #[tokio::test]
    async fn test_existing_summary_is_included_in_the_prompt() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        store
            .set_summary(conversation.id, "we already talked about cats".to_string())
            .await
            .unwrap();
        for text in ["one", "two", "three"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }
        // load_or_create returns the conversation as it was before set_summary in
        // this test's earlier call, so fetch it fresh to see the summary
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();

        let provider = Arc::new(MockProvider::new().respond_text("updated summary"));
        compact_if_needed(
            &summariser(provider.clone()),
            &store,
            &conversation,
            settings(0, 1),
            crate::types::rough_token_estimate,
        )
        .await
        .expect("should succeed");

        let sent = &provider.requests()[0];
        let sent_text = sent
            .history
            .iter()
            .next()
            .map(Message::text)
            .unwrap_or_default();
        assert!(
            sent_text.contains("we already talked about cats"),
            "the existing summary must reach the model, or it is lost on every compaction pass: \
             {sent_text:?}"
        );
    }

    #[tokio::test]
    async fn test_compact_if_needed_propagates_a_provider_error() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        for text in ["one", "two", "three"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }

        let provider =
            Arc::new(MockProvider::new().respond_error(AiError::Provider("outage".to_string())));
        let result = compact_if_needed(
            &summariser(provider),
            &store,
            &conversation,
            settings(0, 1),
            crate::types::rough_token_estimate,
        )
        .await;

        assert!(result.is_err());

        // and the store must be untouched - a failed compaction should never lose
        // messages it never actually got to summarise
        let history = store.history(conversation.id, None).await.unwrap();
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn test_an_empty_model_response_is_an_error_not_a_silently_empty_summary() {
        let store = InMemorySessionStore::new();
        let conversation = store.load_or_create(&scope(), "companion").await.unwrap();
        for text in ["one", "two", "three"] {
            store
                .append(conversation.id, Message::user(text))
                .await
                .unwrap();
        }

        let provider = Arc::new(MockProvider::new().respond_text(""));
        let result = compact_if_needed(
            &summariser(provider),
            &store,
            &conversation,
            settings(0, 1),
            crate::types::rough_token_estimate,
        )
        .await;

        assert!(
            result.is_err(),
            "an empty summary is worse than no compaction at all, and must not be written"
        );
    }
}
