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
    fn test_user_message_converts_to_a_user_turn() {
        let message: munibot_ai::Message = ChatMessage::user("hi").into();
        assert_eq!(message.role, munibot_ai::Role::User);
        assert_eq!(message.text(), "hi");
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
