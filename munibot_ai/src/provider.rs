//! Provider-agnostic model access for munibot's ai harness.
//!
//! `rig-core` is a dependency of this module and no other. Every other module
//! in this crate speaks [`crate::types`] and the `Provider` trait defined here,
//! never a rig type directly.
//!
//! `rig-core`'s `CompletionModel` trait is not object-safe (it carries
//! associated types, a `Clone` supertrait, and `impl Future` return positions),
//! and 0.41 has no runtime provider-selection helper to lean on. This module's
//! job is to hide both of those facts behind an object-safe trait, so the rest
//! of the harness never has to care. See `docs/notes/ai-preflight-findings.md`
//! for how that was verified.

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use crate::types::{AiError, CompletionRequest, CompletionResponse, ContentBlock, StreamEvent};

pub mod mock;

pub use mock::MockProvider;

/// A source of model completions.
///
/// Every concrete provider (rig-backed or otherwise) and the test double both
/// implement this. It is the only thing the rest of the harness ever sees — no
/// rig type, and no HTTP client, crosses this boundary.
#[async_trait]
pub trait Provider: Send + Sync {
    /// A short, stable identifier such as `"anthropic"`, used in logs and error
    /// messages.
    fn name(&self) -> &str;

    /// Executes one completion request and returns it whole.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AiError>;

    /// Executes one completion request, yielding events as they arrive.
    ///
    /// The default wraps [`Self::complete`] and replays its result as a stream,
    /// so a provider that only implements `complete` is still usable
    /// everywhere a streaming one would be. This is more than the minimal
    /// "one text delta plus done": it converts every content block, including
    /// tool calls, because a non-streaming provider that later gets a
    /// tool-using persona would otherwise silently drop them.
    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, AiError>>, AiError> {
        let response = self.complete(request).await?;
        let stop_reason = response.stop_reason;
        let usage = response.usage;

        let mut events = content_to_stream_events(response.content);
        events.push(StreamEvent::Usage { usage });
        events.push(StreamEvent::Done { stop_reason });

        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

/// Converts a completion's content blocks into the stream events that would
/// have produced them.
///
/// Tool results and images never occur in a fresh assistant completion, so they
/// are dropped rather than mapped to anything.
fn content_to_stream_events(content: Vec<ContentBlock>) -> Vec<StreamEvent> {
    content
        .into_iter()
        .flat_map(|block| -> Vec<StreamEvent> {
            match block {
                ContentBlock::Text { text } => vec![StreamEvent::TextDelta { text }],
                ContentBlock::Thinking { thinking } => {
                    vec![StreamEvent::ThinkingDelta { thinking }]
                }
                ContentBlock::ToolUse {
                    call_id,
                    name,
                    arguments,
                } => vec![
                    StreamEvent::ToolUseStart { call_id, name },
                    StreamEvent::ToolUseDelta {
                        partial_json: arguments.to_string(),
                    },
                    StreamEvent::ToolUseEnd,
                ],
                ContentBlock::ToolResult { .. } | ContentBlock::Image { .. } => vec![],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use serde_json::json;

    use super::*;
    use crate::types::{Message, ModelRef, StopReason, Usage};

    /// A minimal stand-in for a real provider, exercising only `complete`, so
    /// the default `stream` implementation is tested in isolation from any
    /// real backend.
    struct StubProvider {
        response: Result<CompletionResponse, AiError>,
    }

    #[async_trait]
    impl Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AiError> {
            match &self.response {
                Ok(response) => Ok(response.clone()),
                Err(_) => Err(AiError::Provider("stubbed failure".to_string())),
            }
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest::new(
            ModelRef::new("anthropic", "claude-opus-5"),
            vec![Message::user("hi")].into(),
        )
    }

    #[tokio::test]
    async fn test_default_stream_wraps_text_into_delta_and_done() {
        let provider = StubProvider {
            response: Ok(CompletionResponse::new(
                vec![ContentBlock::text("hello")],
                StopReason::EndTurn,
                Usage::new(3, 5),
            )),
        };

        let events: Vec<_> = provider
            .stream(request())
            .await
            .expect("stream should succeed")
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|event| event.expect("no event should error"))
            .collect();

        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta {
                    text: "hello".to_string()
                },
                StreamEvent::Usage {
                    usage: Usage::new(3, 5)
                },
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn
                },
            ],
            "a text-only response should replay as a single delta, its usage, then done"
        );
    }

    #[tokio::test]
    async fn test_default_stream_preserves_tool_calls() {
        let provider = StubProvider {
            response: Ok(CompletionResponse::new(
                vec![ContentBlock::tool_use(
                    "c1",
                    "current_time",
                    json!({"timezone": "UTC"}),
                )],
                StopReason::ToolUse,
                Usage::default(),
            )),
        };

        let events: Vec<_> = provider
            .stream(request())
            .await
            .expect("stream should succeed")
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|event| event.expect("no event should error"))
            .collect();

        assert_eq!(
            events[0],
            StreamEvent::ToolUseStart {
                call_id: "c1".to_string(),
                name: "current_time".to_string()
            },
            "a tool call must not be silently dropped by the default stream wrapper"
        );
        assert!(
            matches!(events[1], StreamEvent::ToolUseDelta { .. }),
            "the tool call arguments should follow as a delta"
        );
        assert_eq!(
            events[2],
            StreamEvent::ToolUseEnd,
            "the tool call should be closed before usage and done"
        );
    }

    #[tokio::test]
    async fn test_default_stream_propagates_complete_errors() {
        let provider = StubProvider {
            response: Err(AiError::Other("boom".to_string())),
        };

        let result = provider.stream(request()).await;

        assert!(
            result.is_err(),
            "a failing complete() must fail stream() before any events are produced"
        );
    }
}
