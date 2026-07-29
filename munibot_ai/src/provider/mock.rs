use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    provider::Provider,
    types::{AiError, CompletionRequest, CompletionResponse, ContentBlock, StopReason, Usage},
};

/// A scripted [`Provider`] for tests.
///
/// Push expected responses in the order the code under test should receive
/// them, then run the code. Every request is recorded, so a test can assert on
/// what the harness actually sent — the system prompt it built, the tools it
/// offered, the history it assembled. Running out of scripted responses is a
/// test bug, not a runtime condition, so it panics with a message naming which
/// request went unanswered rather than returning an error a caller might
/// quietly paper over.
pub struct MockProvider {
    name: String,
    responses: Mutex<VecDeque<Result<CompletionResponse, AiError>>>,
    requests: Mutex<Vec<CompletionRequest>>,
}

impl MockProvider {
    /// Builds an empty mock. Chain `respond_*` calls to script its behaviour.
    pub fn new() -> Self {
        Self {
            name: "mock".to_string(),
            responses: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Overrides the name reported by [`Provider::name`], for tests that check
    /// it specifically.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Queues a plain text response.
    pub fn respond_text(self, text: impl Into<String>) -> Self {
        self.respond(Ok(CompletionResponse::new(
            vec![ContentBlock::text(text)],
            StopReason::EndTurn,
            Usage::default(),
        )))
    }

    /// Queues a response that calls a single tool.
    pub fn respond_tool_use(
        self,
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        self.respond(Ok(CompletionResponse::new(
            vec![ContentBlock::tool_use(call_id, name, arguments)],
            StopReason::ToolUse,
            Usage::default(),
        )))
    }

    /// Queues a failure.
    pub fn respond_error(self, error: AiError) -> Self {
        self.respond(Err(error))
    }

    /// Queues a fully custom response, for cases the convenience constructors
    /// above do not cover (multi-block content, non-default usage, a
    /// truncated `MaxTokens` stop reason, and so on).
    pub fn respond(self, response: Result<CompletionResponse, AiError>) -> Self {
        self.responses.lock().unwrap().push_back(response);
        self
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// How many requests have been received so far.
    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AiError> {
        let request_number = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };

        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                panic!(
                    "MockProvider ran out of scripted responses on request {request_number} - \
                     queue more with respond_text, respond_tool_use, respond_error, or respond \
                     before running the code under test"
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request() -> CompletionRequest {
        CompletionRequest::new(
            crate::types::ModelRef::new("anthropic", "claude-opus-5"),
            vec![crate::types::Message::user("hi")].into(),
        )
    }

    #[tokio::test]
    async fn test_respond_text_is_returned_by_complete() {
        let provider = MockProvider::new().respond_text("hello there");

        let response = provider.complete(request()).await.expect("should succeed");

        assert_eq!(
            response.text(),
            "hello there",
            "the scripted text should come back verbatim"
        );
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }

    #[tokio::test]
    async fn test_respond_tool_use_is_returned_by_complete() {
        let provider =
            MockProvider::new().respond_tool_use("c1", "current_time", json!({"timezone": "UTC"}));

        let response = provider.complete(request()).await.expect("should succeed");

        assert_eq!(
            response.stop_reason,
            StopReason::ToolUse,
            "a tool call should stop the turn there"
        );
        let (call_id, name, _) = response.content[0]
            .as_tool_use()
            .expect("the queued response should be a tool call");
        assert_eq!(call_id, "c1");
        assert_eq!(name, "current_time");
    }

    #[tokio::test]
    async fn test_respond_error_is_returned_by_complete() {
        let provider = MockProvider::new().respond_error(AiError::Config("no key".to_string()));

        let result = provider.complete(request()).await;

        assert!(
            result.is_err(),
            "a queued error should surface from complete()"
        );
    }

    #[tokio::test]
    async fn test_responses_are_returned_in_queued_order() {
        let provider = MockProvider::new()
            .respond_text("first")
            .respond_text("second");

        let first = provider
            .complete(request())
            .await
            .expect("first call should succeed");
        let second = provider
            .complete(request())
            .await
            .expect("second call should succeed");

        assert_eq!(
            first.text(),
            "first",
            "responses must come back FIFO, not LIFO"
        );
        assert_eq!(second.text(), "second");
    }

    #[tokio::test]
    async fn test_requests_are_recorded_in_order() {
        let provider = MockProvider::new().respond_text("a").respond_text("b");

        provider.complete(request()).await.expect("should succeed");
        provider.complete(request()).await.expect("should succeed");

        assert_eq!(
            provider.request_count(),
            2,
            "both requests should have been recorded"
        );
        assert_eq!(provider.requests().len(), 2);
    }

    #[tokio::test]
    #[should_panic(expected = "ran out of scripted responses")]
    async fn test_running_out_of_responses_panics_with_a_clear_message() {
        let provider = MockProvider::new();
        let _ = provider.complete(request()).await;
    }

    #[test]
    fn test_named_overrides_the_reported_name() {
        let provider = MockProvider::new().named("anthropic");
        assert_eq!(
            provider.name(),
            "anthropic",
            "the name override should take effect"
        );
    }

    #[test]
    fn test_default_name_is_mock() {
        let provider = MockProvider::new();
        assert_eq!(
            provider.name(),
            "mock",
            "an unnamed mock should identify itself as such"
        );
    }
}
