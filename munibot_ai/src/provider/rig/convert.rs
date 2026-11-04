//! Pure conversions between our provider-neutral types and rig's.
//!
//! This file is where rig's pre-1.0 churn is absorbed. Every function here is
//! pure and synchronous - no client, no network, so this whole file is
//! unit-testable without a provider key.
//!
//! A conversation only ever holds text and reasoning, so anything else rig can
//! carry (tool calls, images) has no representation here: in a reply it is
//! dropped rather than failing the whole turn.

use rig_core::{
    EmptyListError, OneOrMany, completion::message as rig_message,
    streaming::StreamedAssistantContent,
};

use crate::types::{ContentBlock, Message, Role, StreamEvent, Usage};

/// Converts one of our messages into rig's message type.
///
/// Fails only when a message's content cannot be represented at all: an empty
/// content list (`rig` requires at least one block), or a reasoning block in a
/// user message, which only a model can ever produce.
pub fn to_rig_message(message: &Message) -> Result<rig_message::Message, String> {
    match message.role {
        Role::User => Ok(rig_message::Message::User {
            content: to_rig_user_content(&message.content)?,
        }),
        Role::Assistant => Ok(rig_message::Message::Assistant {
            id: None,
            content: to_rig_assistant_content(&message.content)?,
        }),
    }
}

fn to_rig_user_content(
    content: &[ContentBlock],
) -> Result<OneOrMany<rig_message::UserContent>, String> {
    let items = content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => Ok(rig_message::UserContent::text(text.clone())),
            ContentBlock::Thinking { .. } => {
                Err(format!("{block:?} cannot appear in a user message"))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    OneOrMany::many(items).map_err(empty_message_error)
}

fn to_rig_assistant_content(
    content: &[ContentBlock],
) -> Result<OneOrMany<rig_message::AssistantContent>, String> {
    let items = content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => rig_message::AssistantContent::text(text.clone()),
            ContentBlock::Thinking { thinking } => {
                rig_message::AssistantContent::reasoning(thinking)
            }
        })
        .collect::<Vec<_>>();

    OneOrMany::many(items).map_err(empty_message_error)
}

fn empty_message_error(_: EmptyListError) -> String {
    "a message needs at least one content block".to_string()
}

/// Converts one block of an assistant's response back into our own type, or
/// `None` for anything a conversation has no use for (a tool call, an image).
pub fn from_assistant_content(content: rig_message::AssistantContent) -> Option<ContentBlock> {
    match content {
        rig_message::AssistantContent::Text(text) => Some(ContentBlock::text(text.text)),
        rig_message::AssistantContent::Reasoning(reasoning) => Some(ContentBlock::thinking(
            reasoning.first_text().unwrap_or_default(),
        )),
        rig_message::AssistantContent::ToolCall(_) | rig_message::AssistantContent::Image(_) => {
            None
        }
    }
}

/// Converts rig's usage counters into ours.
///
/// `total_tokens` is dropped, since we compute our own total from the parts.
/// `tool_use_prompt_tokens` is also dropped: rig does not document whether it
/// is already included in `input_tokens` or additional to it, and folding it
/// into `input_tokens` risks double-counting on providers where it is not.
pub fn from_rig_usage(usage: rig_core::completion::Usage) -> Usage {
    Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cached_input_tokens,
        cache_write_tokens: usage.cache_creation_input_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    }
}

/// Converts one chunk of a rig stream into zero or more of our stream events.
///
/// Deliberately does not handle [`StreamedAssistantContent::Final`]: that
/// variant carries the whole stream's aggregated response, which is a
/// stream-level concern for the adapter driving this conversion, not a
/// per-chunk one. Tool calls and [`StreamedAssistantContent::Unknown`] (a
/// provider-native item rig does not model) produce no event.
pub fn from_streamed_content<R>(content: StreamedAssistantContent<R>) -> Vec<StreamEvent> {
    match content {
        StreamedAssistantContent::Text(text) => vec![StreamEvent::TextDelta { text: text.text }],
        StreamedAssistantContent::Reasoning(reasoning) => {
            vec![StreamEvent::ThinkingDelta {
                thinking: reasoning.first_text().unwrap_or_default().to_string(),
            }]
        }
        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
            vec![StreamEvent::ThinkingDelta {
                thinking: reasoning,
            }]
        }
        StreamedAssistantContent::ToolCall { .. }
        | StreamedAssistantContent::ToolCallDelta { .. }
        | StreamedAssistantContent::Final(_)
        | StreamedAssistantContent::Unknown(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // --- to_rig_message ---

    #[test]
    fn test_user_message_converts_to_text_content() {
        let message = Message::user("hello");
        let rig_message = to_rig_message(&message).expect("should convert");

        match rig_message {
            rig_message::Message::User { content } => {
                assert_eq!(content.len(), 1);
            }
            other => panic!("expected a user message, got {other:?}"),
        }
    }

    #[test]
    fn test_assistant_text_converts_to_text_content() {
        let message = Message::assistant("hi!");
        let rig_message::Message::Assistant { content, .. } =
            to_rig_message(&message).expect("should convert")
        else {
            panic!("expected an assistant message");
        };

        assert!(matches!(
            content.first(),
            rig_message::AssistantContent::Text(_)
        ));
    }

    #[test]
    fn test_assistant_thinking_becomes_reasoning() {
        let message = Message::new(Role::Assistant, vec![ContentBlock::thinking("hmm")]);
        let rig_message::Message::Assistant { content, .. } =
            to_rig_message(&message).expect("should convert")
        else {
            panic!("expected an assistant message");
        };

        assert!(matches!(
            content.first(),
            rig_message::AssistantContent::Reasoning(_)
        ));
    }

    #[test]
    fn test_thinking_in_a_user_message_is_rejected() {
        let message = Message::new(Role::User, vec![ContentBlock::thinking("hmm")]);
        assert!(
            to_rig_message(&message).is_err(),
            "reasoning can only ever come from the assistant"
        );
    }

    #[test]
    fn test_a_message_with_no_content_is_rejected() {
        let message = Message::new(Role::User, vec![]);
        assert!(
            to_rig_message(&message).is_err(),
            "rig needs at least one content block"
        );
    }

    // --- from_assistant_content ---

    #[test]
    fn test_from_assistant_content_text() {
        let block = from_assistant_content(rig_message::AssistantContent::text("hi"));
        assert_eq!(block, Some(ContentBlock::text("hi")));
    }

    #[test]
    fn test_from_assistant_content_reasoning() {
        let block = from_assistant_content(rig_message::AssistantContent::reasoning("thinking..."));
        assert_eq!(block, Some(ContentBlock::thinking("thinking...")));
    }

    #[test]
    fn test_from_assistant_content_drops_tool_calls() {
        let tool_call = rig_message::AssistantContent::tool_call("c1", "current_time", json!({}));
        assert_eq!(
            from_assistant_content(tool_call),
            None,
            "a conversation has no tools to answer a call with"
        );
    }

    // --- usage ---

    #[test]
    fn test_from_rig_usage_maps_cache_and_reasoning_fields() {
        let usage = from_rig_usage(rig_core::completion::Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: 3,
            cache_creation_input_tokens: 2,
            tool_use_prompt_tokens: 99,
            reasoning_tokens: 7,
        });

        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(
            usage.cache_read_tokens, 3,
            "cached_input_tokens should map to cache_read_tokens"
        );
        assert_eq!(
            usage.cache_write_tokens, 2,
            "cache_creation_input_tokens should map to cache_write_tokens"
        );
        assert_eq!(
            usage.reasoning_tokens, 7,
            "reasoning_tokens should map straight across"
        );
    }

    // --- from_streamed_content ---

    #[test]
    fn test_streamed_text_becomes_a_text_delta() {
        let events = from_streamed_content::<()>(StreamedAssistantContent::Text(
            rig_message::Text::new("hi"),
        ));
        assert_eq!(events, vec![StreamEvent::TextDelta {
            text: "hi".to_string()
        }]);
    }

    #[test]
    fn test_streamed_tool_calls_produce_no_event() {
        let tool_call = rig_message::ToolCall::new(
            "c1".to_string(),
            rig_message::ToolFunction::new("current_time".to_string(), json!({})),
        );
        let events = from_streamed_content::<()>(StreamedAssistantContent::ToolCall {
            tool_call,
            internal_call_id: "internal-1".to_string(),
        });

        assert_eq!(
            events.len(),
            3,
            "a complete tool call should emit start, delta, and end"
        );
        assert_eq!(events[0], StreamEvent::ToolUseStart {
            call_id: "c1".to_string(),
            name: "current_time".to_string()
        });
        assert!(matches!(events[1], StreamEvent::ToolUseDelta { .. }));
        assert_eq!(events[2], StreamEvent::ToolUseEnd);
    }

    #[test]
    fn test_streamed_tool_call_name_delta_starts_the_call() {
        let events = from_streamed_content::<()>(StreamedAssistantContent::ToolCallDelta {
            id: "c1".to_string(),
            internal_call_id: "internal-1".to_string(),
            content: rig_core::streaming::ToolCallDeltaContent::Name("current_time".to_string()),
        });
        assert_eq!(events, vec![StreamEvent::ToolUseStart {
            call_id: "c1".to_string(),
            name: "current_time".to_string()
        }]);
    }

    #[test]
    fn test_streamed_tool_call_argument_delta_is_a_delta_event() {
        let events = from_streamed_content::<()>(StreamedAssistantContent::ToolCallDelta {
            id: "c1".to_string(),
            internal_call_id: "internal-1".to_string(),
            content: rig_core::streaming::ToolCallDeltaContent::Delta("{\"timezone\":".to_string()),
        });
        assert_eq!(events, vec![StreamEvent::ToolUseDelta {
            partial_json: "{\"timezone\":".to_string()
        }]);
    }

    #[test]
    fn test_streamed_reasoning_becomes_thinking_delta() {
        let events = from_streamed_content::<()>(StreamedAssistantContent::Reasoning(
            rig_message::Reasoning::new("hmm"),
        ));
        assert_eq!(events, vec![StreamEvent::ThinkingDelta {
            thinking: "hmm".to_string()
        }]);
    }

    #[test]
    fn test_streamed_reasoning_delta_becomes_thinking_delta() {
        let events = from_streamed_content::<()>(StreamedAssistantContent::ReasoningDelta {
            id: None,
            reasoning: "hmm".to_string(),
        });
        assert_eq!(events, vec![StreamEvent::ThinkingDelta {
            thinking: "hmm".to_string()
        }]);
    }

    #[test]
    fn test_streamed_unknown_produces_no_event() {
        let events =
            from_streamed_content::<()>(StreamedAssistantContent::Unknown(json!({"x": 1})));
        assert_eq!(
            events,
            Vec::new(),
            "a provider-native passthrough item we do not model should be silently skipped, not \
             error"
        );
    }

    #[test]
    fn test_streamed_final_produces_no_event_here() {
        // Final's usage/stop-reason handling is a stream-level concern for the adapter,
        // not this per-chunk conversion - see the function's doc comment.
        let events = from_streamed_content(StreamedAssistantContent::Final(()));
        assert_eq!(events, Vec::new());
    }
}
