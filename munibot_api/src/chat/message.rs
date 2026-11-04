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
            id: row.id,
            role: munibot_ai::types::Role::from_key(&row.role)
                .map(ChatRole::from)
                .unwrap_or(ChatRole::Assistant),
            content,
            created_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.created_at,
                chrono::Utc,
            )
            .to_rfc3339(),
            attachments,
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_ai::types::ContentBlock;
    use munibot_core::db::models::AiMessage;

    use super::*;

    fn row(role: &str) -> AiMessage {
        AiMessage {
            id: 1,
            conversation_id: 1,
            seq: 0,
            role: role.to_string(),
            content: "[]".to_string(),
            token_count: 0,
            created_at: NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
        }
    }

    #[test]
    fn test_text_blocks_are_joined_into_plain_content() {
        let blocks = vec![ContentBlock::text("hello "), ContentBlock::text("there")];
        let message = ChatMessage::from_row(row("user"), &blocks, Vec::new());
        assert_eq!(message.content, "hello there");
        assert_eq!(message.role, ChatRole::User);
    }

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
