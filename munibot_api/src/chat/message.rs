use serde::{Deserialize, Serialize};

use crate::chat::AttachmentSummary;

/// Who sent one message in a conversation transcript.
///
/// Mirrors `munibot_ai::types::Role`, since that type lives in a server-only
/// dependency of this crate and can't be named from wasm-buildable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[cfg(feature = "server")]
impl From<munibot_ai::types::Role> for ChatRole {
    fn from(role: munibot_ai::types::Role) -> Self {
        match role {
            munibot_ai::types::Role::System => Self::System,
            munibot_ai::types::Role::User => Self::User,
            munibot_ai::types::Role::Assistant => Self::Assistant,
            munibot_ai::types::Role::Tool => Self::Tool,
        }
    }
}

/// One persisted message in a conversation, as loaded for the transcript.
///
/// `content` is already flattened to plain text -- the text blocks of the
/// stored message joined together. A stored message can also carry tool use
/// and tool result blocks, but those are surfaced live as
/// [`crate::chat::ChatEvent::ToolStarted`]/
/// [`crate::chat::ChatEvent::ToolFinished`] while a turn is in flight, not
/// replayed as transcript entries once it's over, so they are dropped here
/// rather than serialized in some other shape nothing renders.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChatMessage {
    pub id: i64,
    pub role: ChatRole,
    pub content: String,
    pub created_at: String,
    /// Every image attached to this message, if any - references only,
    /// fetched directly by the browser from `/attachments/{id}` rather than
    /// carried in `content` itself. Never any for a message stored before
    /// attachments existed at all, the same as any other message with none.
    pub attachments: Vec<AttachmentSummary>,
}

#[cfg(feature = "server")]
impl ChatMessage {
    /// Builds a `ChatMessage` from a stored row, its already-parsed content
    /// blocks, and whatever attachments were linked to it.
    ///
    /// Takes the parsed blocks rather than the row's raw JSON `content`
    /// column directly, since parsing can fail and this constructor has no
    /// error path of its own -- the caller decides how a corrupt row is
    /// handled.
    pub fn from_row(
        row: munibot_core::db::models::AiMessage,
        blocks: &[munibot_ai::types::ContentBlock],
        attachments: Vec<AttachmentSummary>,
    ) -> Self {
        use chrono::DateTime;

        let content = blocks
            .iter()
            .filter_map(|block| match block {
                munibot_ai::types::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

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
