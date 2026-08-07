use dioxus::prelude::*;

use crate::chat::ChatResult;

/// Persists the signed-in user's message in `conversation_id`, returning the
/// new message's own id -- the turn identifier the streaming endpoint
/// (`GET /api/ai/chat/stream`) takes to know which message it should now
/// answer.
///
/// Split from streaming deliberately: SSE is a `GET`, and a pasted code
/// block in a query string would hit url length limits exactly when the
/// coding use case needs it most.
///
/// Takes no `persona` parameter, unlike the shape sketched in the milestone
/// plan: a conversation is bound to one persona at creation
/// (`ai_conversations.persona_id`) with no operation to change it, so there
/// is nothing for a per-message override to mean yet.
///
/// `attachment_ids` links images already uploaded through
/// `upload_attachment` to this message, once it's the message that ended up
/// referencing them -- see that function's own doc comment for why the
/// upload itself is a separate step.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn send_message(
    conversation_id: i64,
    text: String,
    attachment_ids: Vec<i64>,
) -> ChatResult<i64> {
    use munibot_ai::types::{History, Message, Role, rough_token_estimate};
    use munibot_core::db::operations::ai;

    use crate::{chat::ChatError, server_fns::chat::conversation::owned_conversation};

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

    // checked up front, before anything is persisted: linking someone
    // else's (or some other conversation's) attachment should fail the
    // whole send, not leave the message stored with half its images
    // silently missing
    for attachment_id in &attachment_ids {
        let attachment = ai::get_attachment_meta(&pool, *attachment_id)
            .await?
            .ok_or(ChatError::AttachmentNotFound)?;
        if attachment.conversation_id != conversation_id {
            return Err(ChatError::AttachmentNotFound);
        }
    }

    let message = Message::user(text);
    let content = serde_json::to_string(&message.content)
        .map_err(|e| ChatError::from(anyhow::anyhow!("couldn't encode message content :< {e}")))?;
    let token_count =
        i32::try_from(History::from(vec![message.clone()]).token_estimate(rough_token_estimate))
            .unwrap_or(i32::MAX);

    let row = ai::append_message(
        &pool,
        conversation_id,
        Role::User.as_key(),
        &content,
        token_count,
    )
    .await?;

    for attachment_id in attachment_ids {
        ai::link_attachment_to_message(&pool, attachment_id, row.id).await?;
    }

    Ok(row.id)
}
