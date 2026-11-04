use serde::{Deserialize, Serialize};

use crate::types::{
    content::ContentBlock,
    message::History,
    model::{ModelParams, ModelRef},
    usage::Usage,
};

/// Everything needed to ask a model for one response.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompletionRequest {
    /// Which provider and model to ask.
    pub model: ModelRef,
    /// Operator instructions, kept separate from the history because most
    /// providers treat the system prompt as a distinct field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// The conversation so far.
    pub history: History,
    #[serde(default)]
    pub params: ModelParams,
}

impl CompletionRequest {
    /// Builds a request with no system prompt or parameter overrides.
    pub fn new(model: ModelRef, history: History) -> Self {
        Self {
            model,
            system: None,
            history,
            params: ModelParams::default(),
        }
    }

    /// Sets the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Sets sampling and length parameters.
    pub fn with_params(mut self, params: ModelParams) -> Self {
        self.params = params;
        self
    }
}

/// Why the model stopped generating.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    EndTurn,
    /// The response hit the token ceiling and is truncated.
    MaxTokens,
    /// A configured stop sequence was produced.
    StopSequence,
    /// The model declined to answer.
    Refusal,
}

/// One response from a model.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompletionResponse {
    /// The blocks the model produced, which may mix reasoning and text.
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

impl CompletionResponse {
    /// Builds a response.
    pub fn new(content: Vec<ContentBlock>, stop_reason: StopReason, usage: Usage) -> Self {
        Self {
            content,
            stop_reason,
            usage,
        }
    }

    /// Concatenates every text block, ignoring reasoning.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join("")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{message::Message, model::ModelRef};

    fn model() -> ModelRef {
        ModelRef::new("anthropic", "claude-opus-5")
    }

    #[test]
    fn test_request_roundtrips() {
        let request = CompletionRequest::new(model(), History::from(vec![Message::user("hi")]))
            .with_system("be nice");

        let encoded = serde_json::to_string(&request).expect("should serialize");
        let decoded: CompletionRequest =
            serde_json::from_str(&encoded).expect("should deserialize");

        assert_eq!(decoded, request, "request should survive a roundtrip");
    }

    #[test]
    fn test_request_omits_an_absent_system_prompt() {
        let request = CompletionRequest::new(model(), History::new());
        let encoded = serde_json::to_value(&request).expect("should serialize");

        assert!(
            encoded.get("system").is_none(),
            "an absent system prompt should not be sent"
        );
    }

    #[test]
    fn test_request_builders_accumulate() {
        let request = CompletionRequest::new(model(), History::new())
            .with_system("s")
            .with_params(ModelParams::new().with_temperature(0.5));

        assert_eq!(request.system.as_deref(), Some("s"), "system should be set");
        assert_eq!(
            request.params.temperature,
            Some(0.5),
            "temperature should be set"
        );
    }

    #[test]
    fn test_response_text_skips_reasoning() {
        let response = CompletionResponse::new(
            vec![
                ContentBlock::thinking("hmm"),
                ContentBlock::text("hello "),
                ContentBlock::text("world"),
            ],
            StopReason::EndTurn,
            Usage::default(),
        );
        assert!(!response.has_tool_uses(), "a text answer has no tool calls");
        assert!(
            response.tool_uses().is_empty(),
            "there should be nothing to iterate"
        );
    }

    #[test]
    fn test_only_tool_use_continues_the_loop() {
        assert!(
            StopReason::ToolUse.wants_another_iteration(),
            "a tool call must be answered, so the loop continues"
        );

        for reason in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::StopSequence,
            StopReason::Refusal,
        ] {
            assert!(
                !reason.wants_another_iteration(),
                "{reason:?} should end the loop"
            );
        }
    }

    #[test]
    fn test_stop_reason_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&StopReason::EndTurn).expect("should serialize");
        assert_eq!(encoded, "\"end_turn\"", "stop reasons should be snake case");
    }

    #[test]
    fn test_response_usage_defaults_when_absent() {
        // some providers omit usage on streamed or cached responses
        let response: CompletionResponse = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn"
        }))
        .expect("should deserialize without usage");
        assert_eq!(
            response.usage,
            Usage::default(),
            "missing usage should be zero"
        );
    }
}
