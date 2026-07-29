use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who authored a piece of conversation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Operator instructions. Outranks every other role.
    System,
    /// The human talking to munibot. Always treated as untrusted input.
    User,
    /// munibot's own output, including tool calls it wants to make.
    Assistant,
    /// Results handed back to the model after it called a tool.
    Tool,
}

/// Where an image's bytes come from.
///
/// Providers disagree on which forms they accept, so both are represented and
/// the provider adapter converts or rejects as needed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageSource {
    /// Base64-encoded image bytes.
    Base64 { data: String },
    /// A URL the provider is expected to fetch.
    Url { url: String },
}

/// An image attached to a message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// An IANA media type such as `image/png`.
    pub media_type: String,
    pub source: ImageSource,
}

/// One piece of a message.
///
/// A single message can mix several blocks: an assistant turn commonly contains
/// reasoning, then text, then one or more tool calls.
///
/// The representation is internally tagged to match how providers wire these on
/// the network, which keeps the conversion layer in the `provider` module close
/// to a field rename.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text { text: String },
    /// An image.
    Image { image: Image },
    /// The model asking to call a tool.
    ToolUse {
        /// Correlates this call with its eventual [`ContentBlock::ToolResult`].
        call_id: String,
        name: String,
        arguments: Value,
    },
    /// The outcome of a tool call, handed back to the model.
    ToolResult {
        /// Must match the `call_id` of the [`ContentBlock::ToolUse`] this
        /// answers.
        call_id: String,
        content: String,
        /// Whether the tool failed. Errors are surfaced to the model so it can
        /// recover, rather than aborting the turn.
        #[serde(default)]
        is_error: bool,
    },
    /// Structured reasoning, where the provider exposes it.
    Thinking { thinking: String },
}

impl ContentBlock {
    /// Builds a text block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Builds a reasoning block.
    pub fn thinking(thinking: impl Into<String>) -> Self {
        Self::Thinking {
            thinking: thinking.into(),
        }
    }

    /// Builds a tool call block.
    pub fn tool_use(call_id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self::ToolUse {
            call_id: call_id.into(),
            name: name.into(),
            arguments,
        }
    }

    /// Builds a successful tool result block.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    /// Builds a failed tool result block.
    ///
    /// The message is shown to the model so that it can correct itself and
    /// retry.
    pub fn tool_error(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            content: content.into(),
            is_error: true,
        }
    }

    /// Returns the text of a [`ContentBlock::Text`], or `None` for any other
    /// block.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Returns the call identifier, name, and arguments of a tool call, or
    /// `None` otherwise.
    pub fn as_tool_use(&self) -> Option<(&str, &str, &Value)> {
        match self {
            Self::ToolUse {
                call_id,
                name,
                arguments,
            } => Some((call_id, name, arguments)),
            _ => None,
        }
    }

    /// Returns `true` if this block is a tool call.
    pub fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse { .. })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn roundtrip(block: &ContentBlock) -> ContentBlock {
        let encoded = serde_json::to_string(block).expect("block should serialize");
        serde_json::from_str(&encoded).expect("block should deserialize")
    }

    #[test]
    fn test_text_block_roundtrips() {
        let block = ContentBlock::text("hello");
        assert_eq!(
            roundtrip(&block),
            block,
            "text block should survive a roundtrip"
        );
    }

    #[test]
    fn test_tool_use_block_roundtrips() {
        let block = ContentBlock::tool_use("call_1", "current_time", json!({"timezone": "UTC"}));
        assert_eq!(
            roundtrip(&block),
            block,
            "tool use block should survive a roundtrip"
        );
    }

    #[test]
    fn test_tool_result_block_roundtrips() {
        let block = ContentBlock::tool_result("call_1", "12:00");
        assert_eq!(
            roundtrip(&block),
            block,
            "tool result block should survive a roundtrip"
        );
    }

    #[test]
    fn test_image_block_roundtrips() {
        let block = ContentBlock::Image {
            image: Image {
                media_type: "image/png".to_string(),
                source: ImageSource::Base64 {
                    data: "iVBORw0KGgo=".to_string(),
                },
            },
        };
        assert_eq!(
            roundtrip(&block),
            block,
            "image block should survive a roundtrip"
        );
    }

    #[test]
    fn test_content_block_is_internally_tagged() {
        let encoded = serde_json::to_value(ContentBlock::text("hi")).expect("should serialize");
        assert_eq!(
            encoded,
            json!({"type": "text", "text": "hi"}),
            "text block should use provider-style internal tagging"
        );
    }

    #[test]
    fn test_tool_result_is_error_defaults_to_false() {
        // providers and stored rows omit the flag on success, so it must not be
        // required
        let block: ContentBlock =
            serde_json::from_value(json!({"type": "tool_result", "call_id": "c", "content": "ok"}))
                .expect("should deserialize without is_error");
        assert_eq!(
            block,
            ContentBlock::tool_result("c", "ok"),
            "missing is_error should mean success"
        );
    }

    #[test]
    fn test_tool_error_marks_the_block_as_failed() {
        let block = ContentBlock::tool_error("c", "no such timezone");
        match block {
            ContentBlock::ToolResult { is_error, .. } => {
                assert!(is_error, "tool_error should set is_error")
            }
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    #[test]
    fn test_accessors_only_match_their_own_variant() {
        let text = ContentBlock::text("hi");
        let call = ContentBlock::tool_use("c", "t", json!({}));

        assert_eq!(text.as_text(), Some("hi"), "text should expose its text");
        assert!(call.as_text().is_none(), "tool call should not expose text");
        assert!(
            text.as_tool_use().is_none(),
            "text should not expose a tool call"
        );
        assert!(call.is_tool_use(), "tool call should report itself as one");
        assert!(
            !text.is_tool_use(),
            "text should not report itself as a tool call"
        );
    }

    #[test]
    fn test_role_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&Role::Assistant).expect("should serialize");
        assert_eq!(
            encoded, "\"assistant\"",
            "roles should be snake case on the wire"
        );
    }
}
