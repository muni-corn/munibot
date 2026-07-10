use dioxus::prelude::*;

use crate::api::auth::{AuthResult, UserData};

/// Returns the currently signed-in user's data, or `None` if no session is
/// active.
///
/// The `AuthSession` extractor is referenced by its full path rather than a
/// top-level `use`, since `crate::api::auth::server` only exists when the
/// `server` feature is on -- a plain `use` of it would fail to resolve when
/// compiling the wasm client, which never enables that feature.
#[server(auth: crate::api::auth::server::AuthSession)]
pub async fn get_authenticated_user() -> AuthResult<Option<UserData>> {
    Ok(auth.current_user.map(|u| u.data.clone()))
}
