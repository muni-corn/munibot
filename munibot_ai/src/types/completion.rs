use serde::{Deserialize, Serialize};

use crate::types::{
    content::ContentBlock,
    message::History,
    model::{ModelParams, ModelRef},
    tool::ToolSchema,
    usage::Usage,
};

/// How strongly the model should be pushed toward calling a tool.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ToolChoice {
    /// The model decides. The right answer almost always.
    #[default]
    Auto,
    /// Tools are advertised but must not be called.
    None,
    /// The model must call some tool before answering.
    Required,
    /// The model must call one of these specific tools.
    Specific { names: Vec<String> },
}

/// Everything needed to ask a model for one response.
///
/// This is a single round trip, not a whole turn. The agent loop in
/// `munibot_ai_harness` builds one of these per iteration, appending tool
/// results to the history as it goes.
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
    /// Tools the model may call. Empty means none are offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSchema>,
    #[serde(default)]
    pub params: ModelParams,
    #[serde(default)]
    pub tool_choice: ToolChoice,
}

impl CompletionRequest {
    /// Builds a request with no system prompt, tools, or parameter overrides.
    pub fn new(model: ModelRef, history: History) -> Self {
        Self {
            model,
            system: None,
            history,
            tools: Vec::new(),
            params: ModelParams::default(),
            tool_choice: ToolChoice::default(),
        }
    }

    /// Sets the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Offers a set of tools.
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    /// Sets sampling and length parameters.
    pub fn with_params(mut self, params: ModelParams) -> Self {
        self.params = params;
        self
    }

    /// Constrains which tools may be called.
    pub fn with_tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = tool_choice;
        self
    }
}

/// Why the model stopped generating.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    EndTurn,
    /// The model wants to call one or more tools. The loop must continue.
    ToolUse,
    /// The response hit the token ceiling and is truncated.
    MaxTokens,
    /// A configured stop sequence was produced.
    StopSequence,
    /// The model declined to answer.
    Refusal,
}

impl StopReason {
    /// Returns `true` when the agent loop must run another iteration.
    pub fn wants_another_iteration(&self) -> bool {
        matches!(self, Self::ToolUse)
    }
}

/// One response from a model.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompletionResponse {
    /// The blocks the model produced, which may mix reasoning, text, and tool
    /// calls.
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

    /// Concatenates every text block, ignoring reasoning and tool calls.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join("")
    }

    /// Borrows every tool call the model asked for, in the order it asked.
    pub fn tool_uses(&self) -> Vec<&ContentBlock> {
        self.content
            .iter()
            .filter(|block| block.is_tool_use())
            .collect()
    }

    /// Returns `true` if the model asked for at least one tool call.
    pub fn has_tool_uses(&self) -> bool {
        self.content.iter().any(ContentBlock::is_tool_use)
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
            .with_system("be nice")
            .with_tools(vec![ToolSchema::no_arguments("ping", "Do nothing.")]);

        let encoded = serde_json::to_string(&request).expect("should serialize");
        let decoded: CompletionRequest =
            serde_json::from_str(&encoded).expect("should deserialize");

        assert_eq!(decoded, request, "request should survive a roundtrip");
    }

    #[test]
    fn test_request_omits_empty_optional_fields() {
        let request = CompletionRequest::new(model(), History::new());
        let encoded = serde_json::to_value(&request).expect("should serialize");

        assert!(
            encoded.get("system").is_none(),
            "an absent system prompt should not be sent"
        );
        assert!(
            encoded.get("tools").is_none(),
            "an empty tool list should not be sent, since some providers reject it"
        );
    }

    #[test]
    fn test_request_builders_accumulate() {
        let request = CompletionRequest::new(model(), History::new())
            .with_system("s")
            .with_params(ModelParams::new().with_temperature(0.5))
            .with_tool_choice(ToolChoice::Required);

        assert_eq!(request.system.as_deref(), Some("s"), "system should be set");
        assert_eq!(
            request.params.temperature,
            Some(0.5),
            "temperature should be set"
        );
        assert_eq!(
            request.tool_choice,
            ToolChoice::Required,
            "tool choice should be set"
        );
    }

    #[test]
    fn test_tool_choice_defaults_to_auto() {
        assert_eq!(
            ToolChoice::default(),
            ToolChoice::Auto,
            "letting the model decide is the right default"
        );
    }

    #[test]
    fn test_tool_choice_specific_roundtrips() {
        let choice = ToolChoice::Specific {
            names: vec!["handoff".to_string()],
        };
        let encoded = serde_json::to_string(&choice).expect("should serialize");
        let decoded: ToolChoice = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(
            decoded, choice,
            "specific tool choice should survive a roundtrip"
        );
    }

    #[test]
    fn test_response_text_skips_non_text_blocks() {
        let response = CompletionResponse::new(
            vec![
                ContentBlock::thinking("hmm"),
                ContentBlock::text("hello "),
                ContentBlock::text("world"),
                ContentBlock::tool_use("c", "ping", json!({})),
            ],
            StopReason::ToolUse,
            Usage::new(1, 1),
        );
        assert_eq!(
            response.text(),
            "hello world",
            "text should skip reasoning and tool calls"
        );
    }

    #[test]
    fn test_response_reports_tool_uses_in_order() {
        let response = CompletionResponse::new(
            vec![
                ContentBlock::tool_use("c1", "first", json!({})),
                ContentBlock::text("thinking out loud"),
                ContentBlock::tool_use("c2", "second", json!({})),
            ],
            StopReason::ToolUse,
            Usage::default(),
        );

        let names: Vec<_> = response
            .tool_uses()
            .iter()
            .filter_map(|block| block.as_tool_use().map(|(_, name, _)| name))
            .collect();

        assert_eq!(
            names,
            vec!["first", "second"],
            "call order must be preserved"
        );
        assert!(
            response.has_tool_uses(),
            "response should report having tool calls"
        );
    }

    #[test]
    fn test_response_without_tool_uses_reports_none() {
        let response = CompletionResponse::new(
            vec![ContentBlock::text("done")],
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
