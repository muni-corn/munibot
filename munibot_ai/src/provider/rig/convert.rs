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

    // --- to_tool_definition ---

    #[test]
    fn test_to_tool_definition_maps_fields_directly() {
        let schema = ToolSchema::new("ping", "does nothing", json!({"type": "object"}));
        let definition = to_tool_definition(&schema);

        assert_eq!(definition.name, "ping");
        assert_eq!(definition.description, "does nothing");
        assert_eq!(definition.parameters, json!({"type": "object"}));
    }

    // --- to_rig_message ---

    #[test]
    fn test_system_message_concatenates_text() {
        let message = Message::system("be nice");
        let rig_message = to_rig_message(&message).expect("should convert");

        match rig_message {
            rig_message::Message::System { content } => assert_eq!(content, "be nice"),
            other => panic!("expected a system message, got {other:?}"),
        }
    }

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
    fn test_tool_role_message_becomes_a_rig_user_message() {
        // rig has no distinct tool role - this is the one non-obvious mapping in the
        // whole file
        let message = Message::tool_results(vec![ContentBlock::tool_result("c1", "12:00")]);
        let rig_message = to_rig_message(&message).expect("should convert");

        assert!(
            matches!(rig_message, rig_message::Message::User { .. }),
            "a Role::Tool message must become a rig User message, not error or vanish"
        );
    }

    #[test]
    fn test_tool_result_error_is_prefixed_since_rig_has_no_is_error_field() {
        let message = Message::tool_results(vec![ContentBlock::tool_error("c1", "not found")]);
        let rig_message::Message::User { content } =
            to_rig_message(&message).expect("should convert")
        else {
            panic!("expected a user message");
        };

        let rig_message::UserContent::ToolResult(result) = content.first() else {
            panic!("expected a tool result");
        };
        let rig_message::ToolResultContent::Text(text) = result.content.first() else {
            panic!("expected text content");
        };
        assert_eq!(
            text.text, "Error: not found",
            "an error result must be recoverable from plain text alone, since rig has no is_error \
             field"
        );
    }

    #[test]
    fn test_tool_result_populates_both_id_and_call_id() {
        let message = Message::tool_results(vec![ContentBlock::tool_result("c1", "ok")]);
        let rig_message::Message::User { content } =
            to_rig_message(&message).expect("should convert")
        else {
            panic!("expected a user message");
        };

        let rig_message::UserContent::ToolResult(result) = content.first() else {
            panic!("expected a tool result");
        };
        assert_eq!(result.id, "c1");
        assert_eq!(
            result.call_id,
            Some("c1".to_string()),
            "both id and call_id should carry our value, since providers disagree on which they \
             read"
        );
    }

    #[test]
    fn test_assistant_tool_use_populates_both_id_and_call_id() {
        let message = Message::new(Role::Assistant, vec![ContentBlock::tool_use(
            "c1",
            "current_time",
            json!({}),
        )]);
        let rig_message::Message::Assistant { content, .. } =
            to_rig_message(&message).expect("should convert")
        else {
            panic!("expected an assistant message");
        };

        let rig_message::AssistantContent::ToolCall(tool_call) = content.first() else {
            panic!("expected a tool call");
        };
        assert_eq!(tool_call.id, "c1");
        assert_eq!(tool_call.call_id, Some("c1".to_string()));
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
    fn test_tool_use_in_a_user_message_is_rejected() {
        let message = Message::new(Role::User, vec![ContentBlock::tool_use(
            "c1",
            "current_time",
            json!({}),
        )]);
        assert!(
            to_rig_message(&message).is_err(),
            "a tool call can only ever come from the assistant"
        );
    }

    #[test]
    fn test_tool_result_in_an_assistant_message_is_rejected() {
        let message = Message::new(Role::Assistant, vec![ContentBlock::tool_result("c1", "ok")]);
        assert!(
            to_rig_message(&message).is_err(),
            "a tool result can only ever come from a user or tool role"
        );
    }

    // --- from_assistant_content ---

    #[test]
    fn test_from_assistant_content_text() {
        let block = from_assistant_content(rig_message::AssistantContent::text("hi"));
        assert_eq!(block, ContentBlock::text("hi"));
    }

    #[test]
    fn test_from_assistant_content_prefers_call_id_over_id() {
        let tool_call = rig_message::AssistantContent::tool_call_with_call_id(
            "internal-id",
            "provider-call-id".to_string(),
            "current_time",
            json!({}),
        );
        let block = from_assistant_content(tool_call);
        let (call_id, ..) = block.as_tool_use().expect("should be a tool call");
        assert_eq!(
            call_id, "provider-call-id",
            "call_id should win over id when both are present"
        );
    }

    #[test]
    fn test_from_assistant_content_falls_back_to_id_when_call_id_absent() {
        let tool_call =
            rig_message::AssistantContent::tool_call("internal-id", "current_time", json!({}));
        let block = from_assistant_content(tool_call);
        let (call_id, ..) = block.as_tool_use().expect("should be a tool call");
        assert_eq!(
            call_id, "internal-id",
            "id should be used when the provider has no separate call_id"
        );
    }

    #[test]
    fn test_from_assistant_content_reasoning() {
        let block = from_assistant_content(rig_message::AssistantContent::reasoning("thinking..."));
        assert_eq!(block, ContentBlock::thinking("thinking..."));
    }

    // --- image roundtrip ---

    #[test]
    fn test_image_base64_roundtrips() {
        let image = Image {
            media_type: "image/png".to_string(),
            source: ImageSource::Base64 {
                data: "abc123".to_string(),
            },
        };
        let rig_image = to_rig_image(&image);
        let block = from_rig_image(rig_image);

        assert_eq!(block, ContentBlock::Image { image });
    }

    #[test]
    fn test_image_url_roundtrips() {
        let image = Image {
            media_type: "image/jpeg".to_string(),
            source: ImageSource::Url {
                url: "https://example.com/cat.jpg".to_string(),
            },
        };
        let rig_image = to_rig_image(&image);
        let block = from_rig_image(rig_image);

        assert_eq!(block, ContentBlock::Image { image });
    }

    #[test]
    fn test_unsupported_rig_image_source_falls_back_to_text() {
        let image = rig_message::Image {
            data: rig_message::DocumentSourceKind::FileId("file-123".to_string()),
            media_type: None,
            detail: None,
            additional_params: None,
        };
        let block = from_rig_image(image);
        assert!(
            block.as_text().is_some(),
            "an unsupported source kind should degrade to a text placeholder, not panic or vanish"
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
    fn test_streamed_complete_tool_call_emits_start_delta_end() {
        let tool_call = rig_message::ToolCall::new(
            "c1".to_string(),
            rig_message::ToolFunction::new("current_time".to_string(), json!({"timezone": "UTC"})),
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
