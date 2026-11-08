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

    let reply = chat
        .reply(messages.into_iter().map(Into::into).collect())
        .map_err(ChatError::from)?;

    let events = ChatEvent::stream_from_reply(reply).map(Ok::<ChatEvent, axum::BoxError>);

    Ok(dioxus_fullstack::ServerEvents::from_stream(events))
}
