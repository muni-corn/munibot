use dioxus::prelude::*;

use crate::chat::{ChatMessage, ChatResult, ConversationSummary};

/// Checks that `owner_user_id` (as stored on a loaded conversation row)
/// matches the signed-in `user_id`, independent of any database access so
/// this security-relevant branch can be unit-tested without one.
///
/// A conversation with no owner at all (a channel-scoped one, which the web
/// chat surface should never be loading in the first place) is never
/// anyone's -- it fails this check the same as one owned by someone else.
#[cfg(feature = "server")]
fn check_ownership(owner_user_id: Option<i64>, user_id: i64) -> ChatResult<()> {
    use crate::chat::ChatError;

    if owner_user_id == Some(user_id) {
        Ok(())
    } else {
        Err(ChatError::NotYourConversation)
    }
}

/// Loads a conversation and verifies the signed-in user owns it.
///
/// Every function in this module (and `super::message`, for sending into an
/// existing conversation) that acts on an existing conversation id calls
/// this first. `ConversationNotFound` and `NotYourConversation` both
/// eventually render as a 404 (see `ChatError::as_status_code`), so a caller
/// can never use the difference to discover whether an id they don't own
/// exists at all.
#[cfg(feature = "server")]
pub(super) async fn owned_conversation(
    pool: &munibot_core::db::DbPool,
    conversation_id: i64,
    user_id: i64,
) -> ChatResult<munibot_core::db::models::AiConversation> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let conversation = ai::get_conversation(pool, conversation_id)
        .await?
        .ok_or(ChatError::ConversationNotFound)?;

    check_ownership(conversation.owner_user_id, user_id)?;

    Ok(conversation)
}

/// Loads a conversation for `get_ai_transcript`: the conversation's own
/// owner may always read it, and so may any operator, for any conversation,
/// see `Permission::Operator`'s own doc comment for what an operator is
/// trusted with.
///
/// A genuinely different check from [`owned_conversation`], not a variant
/// of it: that one is deliberately owner-only (every other
/// conversation-scoped function - sending a message, renaming, archiving -
/// has no business letting an operator act as someone else), while a
/// transcript read is exactly the audit surface an operator needs.
#[cfg(feature = "server")]
pub(super) async fn owner_or_operator_conversation(
    auth: &crate::auth::server::AuthSession,
    pool: &munibot_core::db::DbPool,
    conversation_id: i64,
) -> ChatResult<munibot_core::db::models::AiConversation> {
    use axum_session_auth::HasPermission;
    use munibot_core::{Permission, db::operations::ai};

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let conversation = ai::get_conversation(pool, conversation_id)
        .await?
        .ok_or(ChatError::ConversationNotFound)?;

    let is_owner = conversation.owner_user_id == Some(user.id);
    let is_operator = user.has(&Permission::Operator.to_string(), &None).await;

    if is_owner || is_operator {
        Ok(conversation)
    } else {
        Err(ChatError::NotYourConversation)
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use crate::chat::ChatError;

    #[test]
    fn test_the_owner_passes() {
        assert!(check_ownership(Some(1), 1).is_ok());
    }

    #[test]
    fn test_someone_else_is_rejected() {
        assert!(matches!(
            check_ownership(Some(1), 2),
            Err(ChatError::NotYourConversation)
        ));
    }

    #[test]
    fn test_an_ownerless_conversation_belongs_to_no_one() {
        assert!(matches!(
            check_ownership(None, 1),
            Err(ChatError::NotYourConversation)
        ));
    }
}

/// Every conversation the signed-in user owns, most recently active first,
/// excluding archived ones.
///
/// Deliberately not guild-gated: unlike `require_guild_admin`, nothing here
/// costs a live discord round trip, and a chat surface must not pay one on
/// every page load.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn list_conversations() -> ChatResult<Vec<ConversationSummary>> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;

    let rows = ai::list_conversations_for_user(&pool, user.id).await?;
    Ok(rows.into_iter().map(ConversationSummary::from).collect())
}

/// Starts a new conversation owned by the signed-in user, bound to
/// `persona_id`.
///
/// `scope_key` is generated rather than derived from anything meaningful:
/// unlike a discord channel, a web conversation has no natural external
/// scope to deduplicate against, and each call here is an explicit request
/// for a brand new conversation, never a "resume this one" lookup.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn create_conversation(persona_id: String) -> ChatResult<ConversationSummary> {
    use munibot_core::db::{models::NewAiConversation, operations::ai};

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;

    let now = chrono::Utc::now().naive_utc();
    let row = ai::create_conversation(&pool, NewAiConversation {
        platform: "web".to_string(),
        scope_key: format!("web:{}", uuid::Uuid::new_v4()),
        persona_id,
        owner_user_id: Some(user.id),
        title: None,
        created_at: now,
        last_active_at: now,
    })
    .await?;

    Ok(row.into())
}

/// One page of a conversation's messages, oldest-first within the page.
///
/// `before_seq` loads the page immediately before that sequence number, for
/// scrolling up through history; omit it to load the most recent page.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_conversation_messages(
    conversation_id: i64,
    before_seq: Option<i32>,
    limit: i64,
) -> ChatResult<Vec<ChatMessage>> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

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
        let attachments = ai::list_attachments_for_message(&pool, row.id)
            .await?
            .into_iter()
            .map(crate::chat::AttachmentSummary::from)
            .collect();
        messages.push(ChatMessage::from_row(row, &blocks, attachments));
    }
    Ok(messages)
}

/// Renames a conversation, returning it as saved.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn rename_conversation(
    conversation_id: i64,
    title: String,
) -> ChatResult<ConversationSummary> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

    ai::rename_conversation(&pool, conversation_id, &title).await?;

    let row = ai::get_conversation(&pool, conversation_id)
        .await?
        .ok_or(ChatError::ConversationNotFound)?;
    Ok(row.into())
}

/// Archives a conversation, hiding it from the sidebar without deleting it.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn archive_conversation(conversation_id: i64) -> ChatResult<()> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

    ai::archive_conversation(&pool, conversation_id).await?;
    Ok(())
}
