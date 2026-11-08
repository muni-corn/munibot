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
impl From<ChatMessage> for munibot_ai::rig::message::Message {
    fn from(message: ChatMessage) -> Self {
        match message.role {
            ChatRole::User => Self::user(message.content),
            ChatRole::Assistant => Self::assistant(message.content),
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use munibot_ai::rig::message::Message;

    use super::*;

    #[test]
    fn test_user_message_converts_to_a_user_turn() {
        let message: Message = ChatMessage::user("hi").into();
        assert_eq!(message, Message::user("hi"));
    }

    #[test]
    fn test_assistant_message_converts_to_an_assistant_turn() {
        let message: Message = ChatMessage::assistant("hello!").into();
        assert_eq!(message, Message::assistant("hello!"));
    }
}
