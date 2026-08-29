use dioxus::prelude::*;

use crate::auth::{AuthResult, LinkedAccountSummary, UserData};

/// Returns the currently signed-in user's data, or `None` if no session is
/// active.
///
/// The `AuthSession` extractor is referenced by its full path rather than a
/// top-level `use`, since `crate::auth::server` only exists when the
/// `server` feature is on -- a plain `use` of it would fail to resolve when
/// compiling the wasm client, which never enables that feature.
#[server(auth: crate::auth::server::AuthSession)]
pub async fn get_authenticated_user() -> AuthResult<Option<UserData>> {
    Ok(auth.current_user.map(|u| u.data.clone()))
}

/// Every provider account linked to the signed-in user, for an account
/// settings page.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn list_linked_accounts() -> AuthResult<Vec<LinkedAccountSummary>> {
    use munibot_core::db::operations;

    use crate::auth::AuthError;

    let user = auth.current_user.ok_or(AuthError::NoAuthSession)?;
    let accounts = operations::list_linked_accounts(&pool, user.id).await?;
    Ok(accounts
        .into_iter()
        .map(LinkedAccountSummary::from)
        .collect())
}

/// Unlinks `provider` from the signed-in user, refusing if it is their
/// only remaining sign-in method - see
/// `munibot_core::db::operations::UnlinkAccountOutcome`'s own doc comment
/// for why. Not an error if that provider was never linked in the first
/// place, the same "nothing to undo" reasoning `forget_memory` documents.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn unlink_account(provider: String) -> AuthResult<()> {
    use munibot_core::db::operations::{self, UnlinkAccountOutcome};

    use crate::auth::AuthError;

    let user = auth.current_user.ok_or(AuthError::NoAuthSession)?;
    match operations::unlink_linked_account(&pool, user.id, &provider).await? {
        UnlinkAccountOutcome::Unlinked | UnlinkAccountOutcome::NotFound => Ok(()),
        UnlinkAccountOutcome::LastRemainingAccount => Err(AuthError::LastRemainingAccount),
    }
}
