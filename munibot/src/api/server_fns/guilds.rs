use dioxus::prelude::*;

// only the server actually constructs an AuthError -- the client's stub
// never runs this function's body, so importing it unconditionally would
// warn as unused when compiling for web
#[cfg(feature = "server")]
use crate::api::auth::AuthError;
use crate::api::{auth::AuthResult, guilds::GuildSummary};

/// Returns the discord guilds (servers) the signed-in user owns or can
/// manage.
///
/// Like `get_authenticated_user`, both extractors are referenced by full
/// path inside the attribute rather than top-level `use`s: `axum` itself is
/// an optional, server-only dependency, so a plain `use axum::...` would
/// fail to resolve when compiling the wasm client.
#[server(
    auth: crate::api::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guilds() -> AuthResult<Vec<GuildSummary>> {
    use munibot_core::db::operations;

    use crate::api::oauth::discord;

    let user = auth.current_user.ok_or(AuthError::NoAuthSession)?;

    let linked_account = operations::get_linked_account(&pool, user.id, "discord")
        .await?
        .ok_or_else(|| anyhow::anyhow!("no discord account linked to this user"))?;

    let guilds = discord::get_current_user_guilds(&linked_account.access_token).await?;

    Ok(guilds
        .into_iter()
        .filter(discord::DiscordGuild::is_administered_by_user)
        .map(|guild| GuildSummary {
            icon_url: guild.icon_url(),
            id: guild.id,
            name: guild.name,
        })
        .collect())
}
