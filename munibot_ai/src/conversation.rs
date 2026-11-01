//! A plain, stateless conversation with a model.
//!
//! [`Chat`] keeps nothing between calls: every call to [`Chat::reply`] is
//! handed the whole conversation so far and streams back the model's next
//! message. Where the history lives, and for how long, is entirely the
//! caller's concern.

use std::sync::Arc;

use futures::{StreamExt, stream::BoxStream};

use crate::{
    provider::{Provider, ProviderResolver},
    types::{AiError, CompletionRequest, History, Message, ModelParams, ModelRef, StreamEvent},
};

/// The system prompt every conversation starts with.
const SYSTEM_PROMPT: &str = include_str!("../prompts/conversation.md");

/// The most tokens one reply may run to.
///
/// Anthropic's api requires a ceiling on every request, and a bounded reply
/// is a sensible default for the other providers too.
const MAX_REPLY_TOKENS: u32 = 4096;

/// A conversation partner backed by one configured model.
pub struct Chat {
    provider: Arc<dyn Provider>,
    model: ModelRef,
}

impl Chat {
    /// Builds a chat service for `model`, resolving its provider from the
    /// environment (for example, `ANTHROPIC_API_KEY` for an `anthropic:`
    /// model).
    ///
    /// # Errors
    ///
    /// Returns [`AiError::Config`] if the provider is unknown or its
    /// credentials aren't set.
    pub fn new(model: ModelRef) -> Result<Self, AiError> {
        let provider = ProviderResolver::new().resolve(&model)?;
        Ok(Self::with_provider(provider, model))
    }

    /// Builds a chat service over an already-constructed provider.
    pub fn with_provider(provider: Arc<dyn Provider>, model: ModelRef) -> Self {
        Self { provider, model }
    }

    /// The model this service talks to.
    pub fn model(&self) -> &ModelRef {
        &self.model
    }

    /// Streams the model's reply to `history`, as chunks of text.
    ///
    /// Reasoning and bookkeeping events from the provider are dropped: only
    /// text the model wants shown reaches the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if `history` is empty, or if the provider refuses the
    /// request outright. Failures that happen mid-reply arrive as an `Err`
    /// item on the returned stream instead.
    pub async fn reply(
        &self,
        history: Vec<Message>,
    ) -> Result<BoxStream<'static, Result<String, AiError>>, AiError> {
        if history.is_empty() {
            return Err(AiError::Other(
                "there's nothing to reply to yet :< send a message first".to_string(),
            ));
        }

        let request = CompletionRequest::new(self.model.clone(), History::from(history))
            .with_system(SYSTEM_PROMPT)
            .with_params(ModelParams::new().with_max_tokens(MAX_REPLY_TOKENS));

        let events = self.provider.stream(request).await?;
        Ok(events
            .filter_map(|event| async move {
                match event {
                    Ok(StreamEvent::TextDelta { text }) => Some(Ok(text)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        provider::MockProvider,
        types::{CompletionResponse, ContentBlock, StopReason, Usage},
    };

    fn model() -> ModelRef {
        ModelRef::new("mock", "mock-1")
    }

    fn chat_with(provider: Arc<MockProvider>) -> Chat {
        Chat::with_provider(provider, model())
    }

    async fn collect(chat: &Chat, history: Vec<Message>) -> Result<String, AiError> {
        let mut stream = chat.reply(history).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?);
        }
        Ok(text)
    }

    #[tokio::test]
    async fn test_reply_streams_the_models_text() {
        let provider = Arc::new(MockProvider::new().respond_text("hi there!"));
        let chat = chat_with(provider);

        let text = collect(&chat, vec![Message::user("hello")]).await.unwrap();

        assert_eq!(text, "hi there!");
    }

    #[tokio::test]
    async fn test_reply_sends_the_system_prompt_and_whole_history() {
        let provider = Arc::new(MockProvider::new().respond_text("ok"));
        let chat = chat_with(provider.clone());

        collect(&chat, vec![
            Message::user("first"),
            Message::assistant("second"),
            Message::user("third"),
        ])
        .await
        .unwrap();

        let requests = provider.requests();
        assert_eq!(requests.len(), 1, "one reply should be one request");
        assert_eq!(requests[0].system.as_deref(), Some(SYSTEM_PROMPT));
        assert_eq!(
            requests[0].history.len(),
            3,
            "the full history should be sent"
        );
        assert_eq!(requests[0].model, model());
    }

    #[tokio::test]
    async fn test_reply_drops_reasoning() {
        let provider = Arc::new(MockProvider::new().respond(Ok(CompletionResponse::new(
            vec![
                ContentBlock::thinking("hmm, let me think"),
                ContentBlock::text("the answer"),
            ],
            StopReason::EndTurn,
            Usage::default(),
        ))));
        let chat = chat_with(provider);

        let text = collect(&chat, vec![Message::user("q")]).await.unwrap();

        assert_eq!(text, "the answer", "reasoning must never reach the caller");
    }

    #[tokio::test]
    async fn test_reply_rejects_an_empty_history() {
        let provider = Arc::new(MockProvider::new());
        let chat = chat_with(provider.clone());

        let result = chat.reply(Vec::new()).await;

        assert!(result.is_err(), "there's nothing to answer");
        assert_eq!(provider.request_count(), 0, "no request should be made");
    }

    #[tokio::test]
    async fn test_reply_propagates_provider_errors() {
        let provider = Arc::new(
            MockProvider::new().respond_error(AiError::Rejected("bad api key".to_string())),
        );
        let chat = chat_with(provider);

        let result = chat.reply(vec![Message::user("hi")]).await;

        assert!(matches!(result, Err(AiError::Rejected(_))));
    }
}
