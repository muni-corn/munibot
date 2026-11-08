//! A plain, stateless conversation with a model.
//!
//! [`Chat`] keeps nothing between calls: every call to [`Chat::reply`] is
//! handed the whole conversation so far and streams back the model's next
//! message. Where the history lives, and for how long, is entirely the
//! caller's concern.

use futures::{StreamExt, future, stream::BoxStream};
use rig_core::{
    DynModel, ProviderError,
    catalog::Catalog,
    completion::{CompletionRequest, Message},
    operation::Completion,
    providers::registry::ConnectError,
    streaming::{Item, StreamEvent},
};

/// The system prompt every conversation starts with.
const SYSTEM_PROMPT: &str = include_str!("../prompts/conversation.md");

/// The most tokens one reply may run to.
///
/// Anthropic's api requires a ceiling on every request, and a bounded reply
/// is a sensible default for the other providers too.
const MAX_REPLY_TOKENS: u64 = 4096;

/// A conversation partner backed by one configured model.
pub struct Chat {
    model: DynModel<Completion>,
}

impl Chat {
    /// Connects to `model`, a rig catalog reference such as
    /// `anthropic/claude-opus-5`, reading its credentials from the
    /// environment (`ANTHROPIC_API_KEY` for an `anthropic/` model, and so
    /// on).
    ///
    /// # Errors
    ///
    /// Returns rig's [`ConnectError`] if the reference isn't `vendor/model`,
    /// names a vendor rig doesn't know, or its key isn't set.
    pub fn connect(model: &str) -> Result<Self, ConnectError> {
        Ok(Self::with_model(Catalog::builtin().connect(model)?))
    }

    /// Builds a chat service over an already-connected model, such as rig's
    /// scripted mock in tests.
    pub fn with_model(model: impl Into<DynModel<Completion>>) -> Self {
        Self {
            model: model.into(),
        }
    }

    /// Streams the model's reply to `history`, as chunks of text.
    ///
    /// Reasoning, tool calls, and provider payloads rig doesn't model are
    /// dropped: only text the model wants shown reaches the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the request can't be sent at all, such as an
    /// empty `history`. Failures that happen mid-reply arrive as an `Err`
    /// item on the returned stream instead.
    #[allow(
        clippy::result_large_err,
        reason = "passes rig's own error through unchanged, as rig's own model calls do"
    )]
    pub fn reply(
        &self,
        history: Vec<Message>,
    ) -> Result<BoxStream<'static, Result<String, ProviderError>>, ProviderError> {
        // rig counts the system prompt as history, so it would happily send a
        // request with nothing to answer
        if history.is_empty() {
            return Err(ProviderError::request(
                "there's nothing to reply to yet :< send a message first",
            ));
        }

        let request = CompletionRequest::from(history)
            .preamble(SYSTEM_PROMPT)
            .max_tokens(MAX_REPLY_TOKENS);

        let events = self.model.stream(request)?;
        Ok(events
            .filter_map(|item| {
                future::ready(match item {
                    Ok(Item::Event(StreamEvent::Text { text, .. })) => Some(Ok(text)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
            })
            .boxed())
    }
}

#[cfg(test)]
mod tests {
    use rig_core::test_utils::{MockCompletionModel, MockError, MockStreamEvent, mock_final};

    use super::*;

    #[allow(clippy::result_large_err, reason = "mirrors Chat::reply")]
    async fn collect(chat: &Chat, history: Vec<Message>) -> Result<String, ProviderError> {
        let mut stream = chat.reply(history)?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?);
        }
        Ok(text)
    }

    fn replying(events: Vec<MockStreamEvent>) -> MockCompletionModel {
        MockCompletionModel::from_stream_turns([events])
    }

    #[tokio::test]
    async fn test_reply_streams_the_models_text() {
        let model = replying(vec![
            MockStreamEvent::text("hi "),
            MockStreamEvent::text("there!"),
            MockStreamEvent::FinalResponse(mock_final(Default::default())),
        ]);
        let chat = Chat::with_model(model);

        let text = collect(&chat, vec![Message::user("hello")]).await.unwrap();

        assert_eq!(text, "hi there!");
    }

    #[tokio::test]
    async fn test_reply_sends_the_system_prompt_and_whole_history() {
        let model = replying(vec![
            MockStreamEvent::text("ok"),
            MockStreamEvent::FinalResponse(mock_final(Default::default())),
        ]);
        let chat = Chat::with_model(model.clone());

        collect(&chat, vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::user("third"),
        ])
        .await
        .unwrap();

        let requests = model.requests();
        assert_eq!(requests.len(), 1, "one reply should be one request");
        let history = &requests[0].chat_history;
        assert_eq!(*history[0], Message::system(SYSTEM_PROMPT));
        assert_eq!(history.len(), 4, "the system prompt then the full history");
        assert_eq!(requests[0].max_tokens, Some(MAX_REPLY_TOKENS));
    }

    #[tokio::test]
    async fn test_reply_drops_reasoning() {
        let model = replying(vec![
            MockStreamEvent::ReasoningDelta {
                id: "reasoning-0".to_string(),
                reasoning: "hmm, let me think".to_string(),
            },
            MockStreamEvent::text("the answer"),
            MockStreamEvent::FinalResponse(mock_final(Default::default())),
        ]);
        let chat = Chat::with_model(model);

        let text = collect(&chat, vec![Message::user("q")]).await.unwrap();

        assert_eq!(text, "the answer", "reasoning must never reach the caller");
    }

    #[tokio::test]
    async fn test_reply_rejects_an_empty_history() {
        let model = MockCompletionModel::from_stream_turns(Vec::<Vec<MockStreamEvent>>::new());
        let chat = Chat::with_model(model.clone());

        let result = collect(&chat, Vec::new()).await;

        assert!(result.is_err(), "there's nothing to answer");
        assert_eq!(model.request_count(), 0, "no request should be made");
    }

    #[tokio::test]
    async fn test_reply_passes_provider_errors_through() {
        let model = replying(vec![
            MockStreamEvent::text("par"),
            MockStreamEvent::Error(MockError::provider("bad api key")),
        ]);
        let chat = Chat::with_model(model);

        let result = collect(&chat, vec![Message::user("hi")]).await;

        assert!(result.is_err(), "a mid-reply failure should surface");
    }
}
