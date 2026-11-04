use serde::{Deserialize, Serialize};

/// Who said a message in a conversation.
///
/// Deliberately has no system or tool role: the browser holds the
/// conversation and sends it back every turn, so anything it can say has to
/// be something a person or the model could legitimately have said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ChatRole {
    User,
    Assistant,
}

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    /// A message from the person chatting.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    /// A message from munibot.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[cfg(feature = "server")]
impl From<ChatMessage> for munibot_ai::Message {
    fn from(message: ChatMessage) -> Self {
        match message.role {
            ChatRole::User => Self::user(message.content),
            ChatRole::Assistant => Self::assistant(message.content),
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    #[test]
    fn test_tool_use_blocks_are_dropped_from_content() {
        let blocks = vec![
            ContentBlock::text("checking..."),
            ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
        ];
        let message = ChatMessage::from_row(row("assistant"), &blocks, Vec::new());
        assert_eq!(
            message.content, "checking...",
            "a tool call in the same message shouldn't leak into the rendered text"
        );
    }

    #[test]
    fn test_every_role_maps_to_its_wire_counterpart() {
        assert_eq!(
            ChatMessage::from_row(row("system"), &[], Vec::new()).role,
            ChatRole::System
        );
        assert_eq!(
            ChatMessage::from_row(row("user"), &[], Vec::new()).role,
            ChatRole::User
        );
        assert_eq!(
            ChatMessage::from_row(row("assistant"), &[], Vec::new()).role,
            ChatRole::Assistant
        );
        assert_eq!(
            ChatMessage::from_row(row("tool"), &[], Vec::new()).role,
            ChatRole::Tool
        );
    }
}
