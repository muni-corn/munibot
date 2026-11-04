use serde::{Deserialize, Serialize};

/// One step of progress through a reply, streamed to the chat page over SSE.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ChatEvent {
    /// A chunk of the reply's text.
    TextDelta(String),
    /// The reply is complete. Without this, a dropped connection would look
    /// exactly like a reply that simply ended.
    Finished,
    /// The reply failed partway through. The text is the error's own
    /// friendly, lowercase message, safe to show as is.
    Failed(String),
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
                kind: ChatFailureKind::from(&error),
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
            message: AiError::Cancelled.to_string(),
            kind: ChatFailureKind::Other,
        });
    }

    #[test]
    fn test_budget_exceeded_maps_to_its_own_kind() {
        let error = AiError::BudgetExceeded {
            limit: "8 iterations".to_string(),
        };
        assert_eq!(
            ChatFailureKind::from(&error),
            ChatFailureKind::BudgetExceeded
        );
    }

    #[test]
    fn test_a_transient_provider_error_maps_to_its_own_kind() {
        assert_eq!(
            ChatFailureKind::from(&AiError::Provider("gateway timeout".to_string())),
            ChatFailureKind::Transient
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::RateLimited { retry_after: None }),
            ChatFailureKind::Transient
        );
    }

    #[test]
    fn test_everything_else_falls_back_to_other() {
        assert_eq!(
            ChatFailureKind::from(&AiError::Cancelled),
            ChatFailureKind::Other
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::Rejected("bad key".to_string())),
            ChatFailureKind::Other
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::Tool("boom".to_string())),
            ChatFailureKind::Other
        );
    }
}
