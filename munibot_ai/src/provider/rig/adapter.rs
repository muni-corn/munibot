use async_stream::stream;
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rig_core::{
    completion::{
        CompletionModel, CompletionRequest as RigCompletionRequest, GetTokenUsage,
        message::{Message as RigMessage, ToolChoice as RigToolChoice},
    },
    one_or_many::OneOrMany,
};

use crate::{
    provider::{Provider, rig::convert},
    types::{
        AiError, CompletionRequest, CompletionResponse, ContentBlock, StopReason, StreamEvent,
        ToolChoice,
    },
};

/// Wraps any rig [`CompletionModel`] as an object-safe [`Provider`].
///
/// Generic over the concrete model type because rig's `CompletionModel` cannot
/// be boxed (it carries associated types, a `Clone` supertrait, and `impl
/// Future` return positions) - erasure happens at our own trait instead, once
/// this adapter sits behind an `Arc<dyn Provider>`.
pub struct RigProvider<M> {
    name: String,
    model: M,
}

impl<M> RigProvider<M> {
    /// Wraps a rig model, reporting `name` from [`Provider::name`].
    pub fn new(name: impl Into<String>, model: M) -> Self {
        Self {
            name: name.into(),
            model,
        }
    }
}

/// Assembles a rig completion request from ours.
///
/// A free function rather than a method: it never touches a concrete model,
/// only translates one request type into another, so it can be unit-tested
/// directly without a fake [`CompletionModel`].
fn build_completion_request(request: CompletionRequest) -> Result<RigCompletionRequest, AiError> {
    let mut messages = Vec::with_capacity(request.history.len() + 1);
    if let Some(system) = &request.system {
        messages.push(RigMessage::system(system.clone()));
    }
    for message in request.history.iter() {
        messages.push(convert::to_rig_message(message).map_err(AiError::Provider)?);
    }
    let chat_history = OneOrMany::many(messages).map_err(|_| {
        AiError::Provider(
            "a completion request needs a system prompt or at least one message".to_string(),
        )
    })?;

    let tools = request
        .tools
        .iter()
        .map(convert::to_tool_definition)
        .collect();

    Ok(RigCompletionRequest {
        // this adapter is already bound to one model at construction time; no per-request override
        model: None,
        // the system prompt goes into chat_history as a leading Message::System instead of the
        // legacy preamble field, which rig itself documents as the preferred representation
        preamble: None,
        chat_history,
        documents: vec![],
        tools,
        temperature: request.params.temperature.map(f64::from),
        max_tokens: request.params.max_tokens.map(u64::from),
        additional_params: None,
        tool_choice: Some(to_rig_tool_choice(&request.tool_choice)),
        output_schema: None,
        record_telemetry_content: false,
    })
}

fn to_rig_tool_choice(choice: &ToolChoice) -> RigToolChoice {
    match choice {
        ToolChoice::Auto => RigToolChoice::Auto,
        ToolChoice::None => RigToolChoice::None,
        ToolChoice::Required => RigToolChoice::Required,
        ToolChoice::Specific { names } => RigToolChoice::Specific {
            function_names: names.clone(),
        },
    }
}

/// Infers a stop reason from content alone: [`StopReason::ToolUse`] if the
/// model asked for any tool call, [`StopReason::EndTurn`] otherwise.
///
/// rig's generic `CompletionResponse<T>` and `StreamingCompletionResponse<T>`
/// expose no normalized finish/stop reason - only the provider-specific raw
/// response does, which would defeat the point of a provider-agnostic type.
/// `MaxTokens`, `StopSequence`, and `Refusal` are therefore not
/// currently derivable through this adapter. This is a third rig API gap
/// alongside the two recorded in `docs/notes/ai-preflight-findings.md`, and is
/// an honest limitation rather than a bug: the harness only ever branches on
/// `ToolUse` versus everything else, so loop correctness is unaffected, though
/// diagnostic precision for a truncated response is lost until a per-provider
/// raw-response inspection is worth the genericity cost.
fn infer_stop_reason(content: &[ContentBlock]) -> StopReason {
    if content.iter().any(ContentBlock::is_tool_use) {
        StopReason::ToolUse
    } else {
        StopReason::EndTurn
    }
}

#[async_trait]
impl<M> Provider for RigProvider<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send,
    M::StreamingResponse: Send,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AiError> {
        let rig_request = build_completion_request(request)?;
        let response = self
            .model
            .completion(rig_request)
            .await
            .map_err(|error| AiError::Provider(error.to_string()))?;

        let content: Vec<ContentBlock> = response
            .choice
            .into_iter()
            .map(convert::from_assistant_content)
            .collect();
        let usage = convert::from_rig_usage(response.usage);
        let stop_reason = infer_stop_reason(&content);

        Ok(CompletionResponse::new(content, stop_reason, usage))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, AiError>>, AiError> {
        let rig_request = build_completion_request(request)?;
        let mut inner = self
            .model
            .stream(rig_request)
            .await
            .map_err(|error| AiError::Provider(error.to_string()))?;

        let events = stream! {
            while let Some(chunk) = inner.next().await {
                match chunk {
                    Ok(content) => {
                        for event in convert::from_streamed_content(content) {
                            yield Ok(event);
                        }
                    }
                    Err(error) => {
                        yield Err(AiError::Provider(error.to_string()));
                        return;
                    }
                }
            }

            // the underlying stream has drained; rig has now populated `choice` and `response`
            // with the fully accumulated content and the raw provider response
            let final_content: Vec<ContentBlock> = inner
                .choice
                .iter()
                .cloned()
                .map(convert::from_assistant_content)
                .collect();
            let stop_reason = infer_stop_reason(&final_content);
            let usage = inner
                .response
                .as_ref()
                .map(|response| convert::from_rig_usage(response.token_usage()))
                .unwrap_or_default();

            yield Ok(StreamEvent::Usage { usage });
            yield Ok(StreamEvent::Done { stop_reason });
        };

        Ok(Box::pin(events))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{History, Message, ModelParams, ModelRef};

    // --- build_completion_request ---

    fn request() -> CompletionRequest {
        CompletionRequest::new(
            ModelRef::new("anthropic", "claude-opus-5"),
            History::from(vec![Message::user("hi")]),
        )
    }

    #[test]
    fn test_system_prompt_becomes_a_leading_message() {
        let request = request().with_system("be nice");
        let rig_request = build_completion_request(request).expect("should build");

        assert_eq!(
            rig_request.chat_history.len(),
            2,
            "the system prompt and the one history message should both be present"
        );
        assert!(
            matches!(rig_request.chat_history.first(), RigMessage::System { .. }),
            "the system prompt must lead the chat history"
        );
    }

    #[test]
    fn test_no_model_override_is_sent() {
        let rig_request = build_completion_request(request()).expect("should build");
        assert_eq!(
            rig_request.model, None,
            "the adapter is bound to one model already; a per-request override would be redundant"
        );
    }

    #[test]
    fn test_tools_are_converted() {
        let request = request().with_tools(vec![crate::types::ToolSchema::no_arguments(
            "ping",
            "does nothing",
        )]);
        let rig_request = build_completion_request(request).expect("should build");

        assert_eq!(rig_request.tools.len(), 1);
        assert_eq!(rig_request.tools[0].name, "ping");
    }

    #[test]
    fn test_params_are_widened_not_truncated() {
        // 0.5 is exactly representable in both f32 and f64, so this proves the widen is
        // present without also asserting on f32-to-f64 rounding behaviour,
        // which is not what this test is for
        let request = request().with_params(
            ModelParams::new()
                .with_temperature(0.5)
                .with_max_tokens(2048),
        );
        let rig_request = build_completion_request(request).expect("should build");

        assert_eq!(rig_request.temperature, Some(0.5_f64));
        assert_eq!(rig_request.max_tokens, Some(2048_u64));
    }

    #[test]
    fn test_tool_choice_required_maps_across() {
        let request = request().with_tool_choice(ToolChoice::Required);
        let rig_request = build_completion_request(request).expect("should build");
        assert_eq!(rig_request.tool_choice, Some(RigToolChoice::Required));
    }

    #[test]
    fn test_tool_choice_specific_maps_names() {
        let request = request().with_tool_choice(ToolChoice::Specific {
            names: vec!["handoff".to_string()],
        });
        let rig_request = build_completion_request(request).expect("should build");
        assert_eq!(
            rig_request.tool_choice,
            Some(RigToolChoice::Specific {
                function_names: vec!["handoff".to_string()]
            })
        );
    }

    #[test]
    fn test_empty_request_is_rejected() {
        let empty =
            CompletionRequest::new(ModelRef::new("anthropic", "claude-opus-5"), History::new());
        assert!(
            build_completion_request(empty).is_err(),
            "a request with no system prompt and no history has nothing to send"
        );
    }

    // --- infer_stop_reason ---

    #[test]
    fn test_infer_stop_reason_tool_use() {
        let content = vec![ContentBlock::tool_use("c1", "current_time", json!({}))];
        assert_eq!(infer_stop_reason(&content), StopReason::ToolUse);
    }

    #[test]
    fn test_infer_stop_reason_end_turn() {
        let content = vec![ContentBlock::text("hi")];
        assert_eq!(infer_stop_reason(&content), StopReason::EndTurn);
    }

    // --- RigProvider against a fake CompletionModel ---

    use futures::stream;
    use rig_core::{
        completion::CompletionError,
        streaming::{RawStreamingChoice, StreamingCompletionResponse, StreamingResult},
    };

    /// A minimal [`CompletionModel`] double. Constructed directly (bypassing
    /// `make`, which is only ever invoked through a real client), so tests
    /// can script exactly one canned outcome without touching the network.
    #[derive(Clone)]
    enum FakeModel {
        Text(String),
        ToolCall {
            id: String,
            name: String,
            arguments: serde_json::Value,
        },
        Error,
    }

    impl CompletionModel for FakeModel {
        type Client = ();
        type Response = ();
        type StreamingResponse = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            FakeModel::Text(String::new())
        }

        async fn completion(
            &self,
            _request: RigCompletionRequest,
        ) -> Result<rig_core::completion::CompletionResponse<Self::Response>, CompletionError>
        {
            match self {
                FakeModel::Text(text) => Ok(rig_core::completion::CompletionResponse {
                    choice: OneOrMany::one(rig_core::completion::message::AssistantContent::text(
                        text.clone(),
                    )),
                    usage: rig_core::completion::Usage::new(),
                    raw_response: (),
                    message_id: None,
                }),
                FakeModel::ToolCall {
                    id,
                    name,
                    arguments,
                } => Ok(rig_core::completion::CompletionResponse {
                    choice: OneOrMany::one(
                        rig_core::completion::message::AssistantContent::tool_call(
                            id.clone(),
                            name.clone(),
                            arguments.clone(),
                        ),
                    ),
                    usage: rig_core::completion::Usage::new(),
                    raw_response: (),
                    message_id: None,
                }),
                FakeModel::Error => Err(CompletionError::ResponseError("boom".to_string())),
            }
        }

        async fn stream(
            &self,
            _request: RigCompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let chunks: Vec<Result<RawStreamingChoice<()>, CompletionError>> = match self {
                FakeModel::Text(text) => vec![
                    Ok(RawStreamingChoice::Message(text.clone())),
                    Ok(RawStreamingChoice::FinalResponse(())),
                ],
                FakeModel::ToolCall { .. } => {
                    vec![Ok(RawStreamingChoice::FinalResponse(()))]
                }
                FakeModel::Error => {
                    vec![Err(CompletionError::ResponseError("boom".to_string()))]
                }
            };
            let boxed: StreamingResult<()> = Box::pin(stream::iter(chunks));
            Ok(StreamingCompletionResponse::stream(boxed))
        }
    }

    fn provider(model: FakeModel) -> RigProvider<FakeModel> {
        RigProvider::new("fake", model)
    }

    #[tokio::test]
    async fn test_complete_converts_text_response() {
        let response = provider(FakeModel::Text("hello".to_string()))
            .complete(request())
            .await
            .expect("should succeed");

        assert_eq!(response.text(), "hello");
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }

    #[tokio::test]
    async fn test_complete_converts_tool_call_response() {
        let response = provider(FakeModel::ToolCall {
            id: "c1".to_string(),
            name: "current_time".to_string(),
            arguments: json!({}),
        })
        .complete(request())
        .await
        .expect("should succeed");

        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert!(response.has_tool_uses());
    }

    #[tokio::test]
    async fn test_complete_propagates_model_errors() {
        let result = provider(FakeModel::Error).complete(request()).await;
        assert!(
            result.is_err(),
            "a model failure should surface as an AiError"
        );
    }

    #[tokio::test]
    async fn test_stream_yields_text_then_usage_then_done() {
        let events: Vec<_> = provider(FakeModel::Text("hi".to_string()))
            .stream(request())
            .await
            .expect("should succeed")
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|event| event.expect("no event should error"))
            .collect();

        assert!(
            matches!(
                events.last(),
                Some(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn
                })
            ),
            "the stream should end with Done, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::TextDelta { .. })),
            "the text chunk should have produced a delta"
        );
    }

    #[tokio::test]
    async fn test_stream_propagates_model_errors_and_stops() {
        let events: Vec<_> = provider(FakeModel::Error)
            .stream(request())
            .await
            .expect("should succeed in constructing the stream")
            .collect::<Vec<_>>()
            .await;

        assert_eq!(
            events.len(),
            1,
            "the stream should stop at the first error, not continue"
        );
        assert!(events[0].is_err());
    }
}
