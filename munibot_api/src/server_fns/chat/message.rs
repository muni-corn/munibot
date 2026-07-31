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
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn send_message(conversation_id: i64, text: String) -> ChatResult<i64> {
    use munibot_ai::types::{History, Message, Role, rough_token_estimate};
    use munibot_core::db::operations::ai;

    use crate::{chat::ChatError, server_fns::chat::conversation::owned_conversation};

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

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

    Ok(row.id)
}
