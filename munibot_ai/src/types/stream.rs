use serde::{Deserialize, Serialize};

use crate::types::{completion::StopReason, usage::Usage};

/// One increment of a streamed response.
///
/// Deliberately mirrors the union of Anthropic's and OpenAI's stream shapes
/// rather than either one alone, so that adapting a provider is a direct
/// mapping instead of a lossy one. A tool call arrives as a start event naming
/// it, zero or more argument deltas as its JSON accumulates, and an
/// end event once the arguments are complete and parseable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// A chunk of assistant-visible text.
    TextDelta { text: String },
    /// A chunk of reasoning, where the provider exposes it incrementally.
    ThinkingDelta { thinking: String },
    /// The model has begun a tool call. Its arguments arrive next as deltas.
    ToolUseStart { call_id: String, name: String },
    /// A chunk of a tool call's JSON arguments, to be concatenated in order.
    ToolUseDelta { partial_json: String },
    /// A tool call's arguments are complete.
    ToolUseEnd,
    /// Token accounting for the response so far. May arrive more than once, and
    /// the latest value wins.
    Usage { usage: Usage },
    /// The response is finished.
    Done { stop_reason: StopReason },
}

impl StreamEvent {
    /// Returns the text of a [`StreamEvent::TextDelta`], or `None` for any
    /// other event.
    pub fn as_text_delta(&self) -> Option<&str> {
        match self {
            Self::TextDelta { text } => Some(text),
            _ => None,
        }
    }

    /// Returns `true` once the stream has produced its terminal event.
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_delta_roundtrips() {
        let event = StreamEvent::TextDelta {
            text: "hi the".to_string(),
        };
        let encoded = serde_json::to_string(&event).expect("should serialize");
        let decoded: StreamEvent = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, event, "text delta should survive a roundtrip");
    }

    #[test]
    fn test_tool_use_lifecycle_roundtrips() {
        for event in [
            StreamEvent::ToolUseStart {
                call_id: "c1".to_string(),
                name: "current_time".to_string(),
            },
            StreamEvent::ToolUseDelta {
                partial_json: "{\"timezone\":".to_string(),
            },
            StreamEvent::ToolUseEnd,
        ] {
            let encoded = serde_json::to_string(&event).expect("should serialize");
            let decoded: StreamEvent = serde_json::from_str(&encoded).expect("should deserialize");
            assert_eq!(
                decoded, event,
                "tool use lifecycle event should roundtrip: {event:?}"
            );
        }
    }

    #[test]
    fn test_events_are_internally_tagged() {
        let encoded = serde_json::to_value(StreamEvent::ToolUseEnd).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!({"type": "tool_use_end"}),
            "a unit-like variant should still carry its type tag"
        );
    }

    #[test]
    fn test_as_text_delta_only_matches_its_own_variant() {
        let text = StreamEvent::TextDelta {
            text: "hi".to_string(),
        };
        let other = StreamEvent::ToolUseEnd;

        assert_eq!(
            text.as_text_delta(),
            Some("hi"),
            "text delta should expose its text"
        );
        assert!(
            other.as_text_delta().is_none(),
            "a non-text event should expose nothing"
        );
    }

    #[test]
    fn test_is_done_only_true_for_done() {
        assert!(
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            }
            .is_done(),
            "a done event should report itself as done"
        );
        assert!(
            !StreamEvent::TextDelta {
                text: "hi".to_string()
            }
            .is_done(),
            "a text delta is not the end of the stream"
        );
    }

    #[test]
    fn test_usage_event_carries_a_usage_record() {
        let event = StreamEvent::Usage {
            usage: Usage::new(10, 5),
        };
        let encoded = serde_json::to_string(&event).expect("should serialize");
        let decoded: StreamEvent = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(
            decoded, event,
            "a usage event should roundtrip with its counts intact"
        );
    }
}
