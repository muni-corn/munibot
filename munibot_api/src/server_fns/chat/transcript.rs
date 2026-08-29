use dioxus::prelude::*;

use crate::chat::{AiTranscript, ChatResult};

/// A conversation's full transcript: every stored message (paginated,
/// oldest-first within the page, the same shape and pagination contract as
/// `get_conversation_messages`) with every tool call audited for it.
///
/// Gated for the conversation's own owner, or any operator, for any
/// conversation - see `owner_or_operator_conversation`'s own doc comment
/// for why this is a genuinely different check from every other
/// conversation-scoped function in this module (owner-only). This is the
/// audit surface behind the memory-wipe promise: an operator needs to be
/// able to see why a persona behaved oddly in *any* conversation, not just
/// their own.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_ai_transcript(
    conversation_id: i64,
    before_seq: Option<i32>,
    limit: i64,
) -> ChatResult<AiTranscript> {
    use munibot_core::db::operations::ai;

    use crate::{
        chat::{ChatError, TranscriptMessage, TranscriptToolCall},
        server_fns::chat::conversation::owner_or_operator_conversation,
    };

    owner_or_operator_conversation(&auth, &pool, conversation_id).await?;

    let rows = ai::get_messages_page(&pool, conversation_id, before_seq, limit).await?;
    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let blocks: Vec<munibot_ai::types::ContentBlock> = serde_json::from_str(&row.content)
            .map_err(|e| {
                ChatError::from(anyhow::anyhow!(
                    "message {} has unparsable content :< {e}",
                    row.id
                ))
            })?;
        messages.push(TranscriptMessage::from_row(row, &blocks));
    }

    let tool_calls = ai::list_tool_calls_for_conversation(&pool, conversation_id)
        .await?
        .into_iter()
        .map(TranscriptToolCall::from)
        .collect();

    Ok(AiTranscript {
        messages,
        tool_calls,
    })
}
