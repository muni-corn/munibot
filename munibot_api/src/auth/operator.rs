use axum_session_auth::HasPermission;
use munibot_core::Permission;

use crate::{
    auth::server::AuthSession,
    chat::{ChatError, ChatResult},
};

/// Verifies that the session's signed-in user holds `Permission::Operator`.
///
/// Every server function that reads service-wide state - not scoped to any
/// one user or guild - must call this first, the same way every guild-scoped
/// settings function calls `crate::auth::guild::require_guild_admin`.
///
/// Takes no `DbPool`, unlike that sibling check: a permission is loaded once
/// into the session at sign-in (see `auth::server::User::load_user`) and
/// checked entirely in memory from then on, never a live query per call.
///
/// Not unit tested directly, the same as `require_guild_admin`: building a
/// real `AuthSession` needs live session-store machinery this crate has no
/// lightweight way to fake. `HasPermission::has`'s own membership check,
/// which is where the actual logic lives, is tested in `auth::server`.
pub async fn require_operator(auth: &AuthSession) -> ChatResult<()> {
    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    if user.has(&Permission::Operator.to_string(), &None).await {
        Ok(())
    } else {
        Err(ChatError::NotOperator)
    }
}
