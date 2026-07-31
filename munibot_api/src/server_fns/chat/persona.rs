use dioxus::prelude::*;

use crate::chat::{ChatResult, PersonaSummary};

/// Every persona available to chat with, so the picker and the future
/// catalogue page share one source of truth rather than each hardcoding
/// their own list.
#[server(
    auth: crate::auth::server::AuthSession,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Ai>>>,
)]
pub async fn list_personas() -> ChatResult<Vec<PersonaSummary>> {
    use crate::chat::ChatError;

    auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let ai_service = ai.0.clone().ok_or(ChatError::AiDisabled)?;

    Ok(ai_service.personas().map(PersonaSummary::from).collect())
}
