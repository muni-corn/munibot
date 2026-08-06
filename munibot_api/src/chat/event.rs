use serde::{Deserialize, Serialize};

/// Tokens consumed by one iteration or a whole turn.
///
/// Less granular than `munibot_ai::types::Usage`: nothing in the chat page
/// shows a cache/reasoning token breakdown, so they're collapsed into
/// `total_tokens` rather than carried across the wire unused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[cfg(feature = "server")]
impl From<munibot_ai::types::Usage> for ChatUsage {
    fn from(usage: munibot_ai::types::Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens(),
        }
    }
}

/// One step of progress through a turn, streamed to the chat page over SSE.
///
/// The serializable mirror of `munibot_ai::harness::HarnessEvent`, which
/// derives only `Debug` and carries an `AiError` and a `serde_json::Value`
/// that must not travel to the client as anything richer than plain text.
/// The `From` impl below is the one genuinely testable part of this mapping.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum ChatEvent {
    /// The turn has begun.
    TurnStarted { persona: String },
    /// A chunk of the model's reasoning, where the provider exposes it.
    Thinking(String),
    /// A chunk of assistant-visible text.
    TextDelta(String),
    /// A tool call has begun.
    ToolStarted { name: String },
    /// A tool call has finished.
    ToolFinished {
        name: String,
        duration_ms: u64,
        ok: bool,
        /// The tool's own success text, recoverable error, or fatal error
        /// message, for the tool activity display's "inspect" affordance.
        /// Not truncated here the way an audit record is: showing it in
        /// full is the entire point of a person explicitly inspecting it.
        result: String,
    },
    /// One provider round trip has finished.
    IterationComplete { iteration: usize, usage: ChatUsage },
    /// The model produced a structured handoff payload.
    Handoff(serde_json::Value),
    /// The turn is over.
    TurnFinished { usage: ChatUsage, cost_micros: i64 },
    /// The turn failed. Carries the error's own friendly, lowercase message
    /// (`AiError`'s `Display` impl) rather than the error type itself, which
    /// isn't `Serialize` and shouldn't cross the wire as anything richer
    /// than the same text a person is already meant to read.
    Failed { message: String },
}

#[cfg(feature = "server")]
impl From<munibot_ai::harness::HarnessEvent> for ChatEvent {
    fn from(event: munibot_ai::harness::HarnessEvent) -> Self {
        use munibot_ai::harness::HarnessEvent;

        match event {
            HarnessEvent::TurnStarted { persona } => Self::TurnStarted { persona },
            HarnessEvent::Thinking(text) => Self::Thinking(text),
            HarnessEvent::TextDelta(text) => Self::TextDelta(text),
            HarnessEvent::ToolStarted { name } => Self::ToolStarted { name },
            HarnessEvent::ToolFinished {
                name,
                duration,
                ok,
                result,
            } => Self::ToolFinished {
                name,
                duration_ms: duration.as_millis() as u64,
                ok,
                result,
            },
            HarnessEvent::IterationComplete { iteration, usage } => Self::IterationComplete {
                iteration,
                usage: usage.into(),
            },
            HarnessEvent::Handoff(payload) => Self::Handoff(payload),
            HarnessEvent::TurnFinished { usage, cost } => Self::TurnFinished {
                usage: usage.into(),
                cost_micros: cost.0,
            },
            HarnessEvent::Failed(error) => Self::Failed {
                message: error.to_string(),
            },
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::time::Duration;

    use munibot_ai::{harness::HarnessEvent, types::AiError};

    use super::*;

    #[test]
    fn test_turn_started_maps_the_persona_name() {
        let event: ChatEvent = HarnessEvent::TurnStarted {
            persona: "companion".to_string(),
        }
        .into();
        assert_eq!(event, ChatEvent::TurnStarted {
            persona: "companion".to_string()
        });
    }

    #[test]
    fn test_thinking_and_text_delta_map_their_own_text() {
        let thinking: ChatEvent = HarnessEvent::Thinking("hmm".to_string()).into();
        assert_eq!(thinking, ChatEvent::Thinking("hmm".to_string()));

        let text: ChatEvent = HarnessEvent::TextDelta("hi".to_string()).into();
        assert_eq!(text, ChatEvent::TextDelta("hi".to_string()));
    }

    #[test]
    fn test_tool_started_maps_the_tool_name() {
        let event: ChatEvent = HarnessEvent::ToolStarted {
            name: "web_search".to_string(),
        }
        .into();
        assert_eq!(event, ChatEvent::ToolStarted {
            name: "web_search".to_string()
        });
    }

    #[test]
    fn test_tool_finished_converts_duration_to_milliseconds() {
        let event: ChatEvent = HarnessEvent::ToolFinished {
            name: "web_search".to_string(),
            duration: Duration::from_millis(250),
            ok: true,
            result: "three results found".to_string(),
        }
        .into();
        assert_eq!(event, ChatEvent::ToolFinished {
            name: "web_search".to_string(),
            duration_ms: 250,
            ok: true,
            result: "three results found".to_string(),
        });
    }

    #[test]
    fn test_iteration_complete_collapses_usage_into_a_total() {
        let event: ChatEvent = HarnessEvent::IterationComplete {
            iteration: 2,
            usage: munibot_ai::types::Usage::new(10, 20),
        }
        .into();
        assert_eq!(event, ChatEvent::IterationComplete {
            iteration: 2,
            usage: ChatUsage {
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
            },
        });
    }

    #[test]
    fn test_handoff_carries_the_payload_verbatim() {
        let event: ChatEvent =
            HarnessEvent::Handoff(serde_json::json!({"action": "ApprovePlan"})).into();
        assert_eq!(
            event,
            ChatEvent::Handoff(serde_json::json!({"action": "ApprovePlan"}))
        );
    }

    #[test]
    fn test_turn_finished_maps_usage_and_cost() {
        let event: ChatEvent = HarnessEvent::TurnFinished {
            usage: munibot_ai::types::Usage::new(5, 5),
            cost: munibot_ai::types::Cost::from_micros(1200),
        }
        .into();
        assert_eq!(event, ChatEvent::TurnFinished {
            usage: ChatUsage {
                input_tokens: 5,
                output_tokens: 5,
                total_tokens: 10,
            },
            cost_micros: 1200,
        });
    }

    #[test]
    fn test_failed_carries_the_error_s_friendly_message_not_the_error_type() {
        let event: ChatEvent = HarnessEvent::Failed(AiError::Cancelled).into();
        assert_eq!(event, ChatEvent::Failed {
            message: AiError::Cancelled.to_string()
        });
    }
}
