use dioxus::prelude::*;
// only the server actually builds a stream or constructs a ChatError -- the
// client's stub never runs this function's body, so importing these
// unconditionally would warn as unused when compiling for web
#[cfg(feature = "server")]
use futures::StreamExt;

#[cfg(feature = "server")]
use crate::chat::ChatError;
use crate::chat::{ChatEvent, ChatResult};

/// The tier a signed-in web user's turns run at, matching what discord
/// grants today (`DISCORD_GRANTED_TIER` in `munibot_discord::handlers::ai`):
/// `Safe` plus read-only network access, nothing that reaches munibot's own
/// data or a sandbox. Revisit once either concept exists for the web
/// surface specifically.
#[cfg(feature = "server")]
const WEB_GRANTED_TIER: munibot_ai::tools::RiskTier = munibot_ai::tools::RiskTier::NetworkRead;

/// Streams the events of one turn answering `message_id`, a turn identifier
/// previously returned by `send_message`.
///
/// `#[server]` hard-codes `POST`, so this is a `#[get]` route instead - the
/// `name: Type` extractor syntax parses identically on it. SSE rather than a
/// websocket: the client sends exactly one message per turn, so duplex buys
/// nothing, while SSE reconnects trivially and is readable in devtools.
#[get(
    "/api/ai/chat/stream?message_id",
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Ai>>>,
)]
pub async fn chat_stream(message_id: i64) -> ChatResult<dioxus_fullstack::ServerEvents<ChatEvent>> {
    use munibot_ai::{
        AiTurnRequest,
        memory::ConversationScope,
        persona::PersonaId,
        tools::Platform,
        types::{ContentBlock, Message, Role},
    };
    use munibot_core::db::operations::ai;
    use tokio_util::sync::CancellationToken;

    use crate::server_fns::chat::conversation::owned_conversation;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let ai_service = ai.0.clone().ok_or(ChatError::AiDisabled)?;

    let message_row = ai::get_message(&pool, message_id)
        .await?
        .ok_or(ChatError::ConversationNotFound)?;
    let conversation = owned_conversation(&pool, message_row.conversation_id, user.id).await?;

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
