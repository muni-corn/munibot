use dioxus::prelude::*;

use crate::chat::{ChatResult, MemoryEntry, MemorySettings};

/// The signed-in user's memory opt-in setting.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_memory_settings() -> ChatResult<MemorySettings> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let row = ai::get_user_settings(&pool, user.id).await?;
    Ok(row.into())
}

/// Sets the signed-in user's memory opt-in setting, returning it as saved.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn set_memory_opt_in(opted_in: bool) -> ChatResult<MemorySettings> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let row = ai::set_memory_opt_in(&pool, user.id, opted_in).await?;
    Ok(Some(row).into())
}

/// Every memory the signed-in user has recorded.
///
/// Ungated by the opt-in flag itself, deliberately: this is the account
/// holder inspecting their own data, not the model reading it back into a
/// prompt (that path is `crate::memory::GatedMemoryStore` in `munibot_ai`,
/// which does check it). A person who has opted out still deserves to see
/// what is stored about them before deciding whether to wipe it.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn list_memories() -> ChatResult<Vec<MemoryEntry>> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let rows = ai::list_memories(&pool, user.id).await?;
    Ok(rows.into_iter().map(MemoryEntry::from).collect())
}

/// Forgets one specific memory. Not an error if it never existed.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn forget_memory(key: String) -> ChatResult<()> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    ai::forget_memory(&pool, user.id, &key).await?;
    Ok(())
}

/// Forgets everything the signed-in user has ever recorded.
///
/// Also ungated by the opt-in flag - see `list_memories` - so someone who
/// opted back out retains the right to delete leftover data without first
/// opting back in.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn wipe_memories() -> ChatResult<()> {
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    ai::wipe_memories(&pool, user.id).await?;
    Ok(())
}
