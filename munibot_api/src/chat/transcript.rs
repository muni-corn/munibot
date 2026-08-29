use serde::{Deserialize, Serialize};

use crate::chat::ChatRole;

/// A transcript's own view of one stored message: like
/// [`crate::chat::ChatMessage`], but with the model's own reasoning
/// ("thinking") blocks stripped rather than silently kept, and always a
/// finished, already-stored message read back for review, never one still
/// streaming in.
///
/// Tool calls are not nested inside this: `ai_tool_calls` (see
/// [`TranscriptToolCall`]) has no `message_id` of its own, only a
/// conversation-scoped timeline, so [`AiTranscript`] carries the two lists
/// separately and a viewer interleaves them by `created_at`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TranscriptMessage {
    pub id: i64,
    pub role: ChatRole,
    pub content: String,
    pub created_at: String,
}

#[cfg(feature = "server")]
impl TranscriptMessage {
    /// Builds a `TranscriptMessage` from a stored row and its already-parsed
    /// content blocks - the same reasoning `ChatMessage::from_row` documents
    /// for taking parsed blocks rather than the row's raw JSON directly.
    pub fn from_row(
        row: munibot_core::db::models::AiMessage,
        blocks: &[munibot_ai::types::ContentBlock],
    ) -> Self {
        use chrono::DateTime;

        let content = blocks
            .iter()
            .filter_map(|block| match block {
                munibot_ai::types::ContentBlock::Text { text } => Some(text.as_str()),
                // reasoning, tool use, tool results, and images are all
                // excluded here - reasoning because this type's whole
                // reason for existing separately from ChatMessage is to
                // strip it, and the rest because they're surfaced as
                // TranscriptToolCall entries (or, for an image, an
                // attachment reference) instead of inline text
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
        }
    }
}

/// One tool call audited during a conversation, as shown in the transcript
/// viewer's own timeline - `ai_tool_calls`, read back for the first time
/// since that table started being written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TranscriptToolCall {
    pub id: i64,
    pub tool_name: String,
    pub input: String,
    pub output: String,
    pub duration_ms: i64,
    /// `"ok"`, `"err"`, or `"fatal"` - mirrors
    /// `munibot_ai::audit::ToolCallStatus::as_key`.
    pub status: String,
    pub created_at: String,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::models::AiToolCall> for TranscriptToolCall {
    fn from(row: munibot_core::db::models::AiToolCall) -> Self {
        use chrono::DateTime;

        Self {
            id: row.id,
            tool_name: row.tool_name,
            input: row.input.unwrap_or_default(),
            output: row.output.unwrap_or_default(),
            duration_ms: row.duration_ms,
            status: row.status,
            created_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.created_at,
                chrono::Utc,
            )
            .to_rfc3339(),
        }
    }
}

/// A conversation's full transcript: its messages (paginated, the same
/// shape `get_conversation_messages` already uses) and every tool call
/// audited for it (not paginated - a conversation's tool-call volume is
/// bounded by its own turn count, which the message pagination already
/// limits per request).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct AiTranscript {
    pub messages: Vec<TranscriptMessage>,
    pub tool_calls: Vec<TranscriptToolCall>,
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_ai::types::ContentBlock;
    use munibot_core::db::models::{AiMessage, AiToolCall};

    use super::*;

    fn message_row(role: &str) -> AiMessage {
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
    fn test_thinking_blocks_are_stripped_from_transcript_content() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "let me consider this carefully".to_string(),
            },
            ContentBlock::text("here's my answer"),
        ];
        let message = TranscriptMessage::from_row(message_row("assistant"), &blocks);
        assert_eq!(message.content, "here's my answer");
    }

    #[test]
    fn test_tool_blocks_are_excluded_from_transcript_content_too() {
        let blocks = vec![
            ContentBlock::text("checking..."),
            ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
        ];
        let message = TranscriptMessage::from_row(message_row("assistant"), &blocks);
        assert_eq!(message.content, "checking...");
    }

    #[test]
    fn test_transcript_tool_call_from_row_defaults_missing_input_output_to_empty() {
        let now =
            NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let call: TranscriptToolCall = AiToolCall {
            id: 1,
            conversation_id: Some(1),
            tool_name: "current_time".to_string(),
            input: None,
            output: None,
            duration_ms: 5,
            status: "ok".to_string(),
            created_at: now,
        }
        .into();

        assert_eq!(call.tool_name, "current_time");
        assert_eq!(call.input, "");
        assert_eq!(call.output, "");
        assert_eq!(call.status, "ok");
    }
}
