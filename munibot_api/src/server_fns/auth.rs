use dioxus::prelude::*;

use crate::auth::{AuthResult, UserData};

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
