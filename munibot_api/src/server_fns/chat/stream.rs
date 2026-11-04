use dioxus::prelude::*;

use crate::chat::{ChatEvent, ChatMessage, ChatResult};

/// Streams munibot's reply to `messages`, the whole conversation so far.
///
/// Nothing is stored server-side: the browser holds the conversation and
/// sends all of it every turn, so a conversation lasts exactly as long as the
/// page that started it. SSE rather than a websocket: the client sends
/// exactly one request per reply, so duplex buys nothing, while SSE is
/// readable in devtools.
///
/// A conversation must end in a message from the person chatting; see
/// `validate_conversation` for the limits it has to stay within.
#[post(
    "/api/ai/chat/stream",
    auth: crate::auth::server::AuthSession,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Chat>>>,
)]
pub async fn chat_stream(
    messages: Vec<ChatMessage>,
) -> ChatResult<dioxus_fullstack::ServerEvents<ChatEvent>> {
    use futures::StreamExt;

    use crate::chat::{ChatError, validate_conversation};

    auth.current_user.as_ref().ok_or(ChatError::NotSignedIn)?;
    let chat = ai.0.clone().ok_or(ChatError::AiDisabled)?;
    validate_conversation(&messages)?;

    let blocks: Vec<ContentBlock> = serde_json::from_str(&message_row.content)
        .map_err(|e| ChatError::from(anyhow::anyhow!("couldn't decode message content :< {e}")))?;
    let text = Message::new(Role::User, blocks).text();

    let request = AiTurnRequest {
        persona_id: PersonaId::new(conversation.persona_id.clone()),
        scope: ConversationScope::new(Platform::Web, conversation.scope_key.clone()),
        user_id: user.id as u64,
        user_name: user.data.display_name.clone(),
        granted_tier: WEB_GRANTED_TIER,
        guild_id: None,
        message: text,
        cancellation: CancellationToken::new(),
        already_persisted: true,
    };

    let events = ai_service
        .turn_streamed(request)
        .await
        .map_err(ChatError::from)?
        .map(|event| Ok::<ChatEvent, axum::BoxError>(ChatEvent::from(event)));

    Ok(dioxus_fullstack::ServerEvents::from_stream(events))
}
