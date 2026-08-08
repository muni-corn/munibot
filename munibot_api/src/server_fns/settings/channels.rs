use dioxus::prelude::*;

use crate::settings::{ChannelSummary, SettingsResult};

/// Returns the text-postable channels of a guild the signed-in user
/// administers, for a channel picker.
///
/// Both extractors are referenced by full path inside the attribute, same
/// as `get_guilds`: `axum` and `munibot_core` are optional, server-only
/// dependencies, so a top-level `use` of either would fail to resolve when
/// compiling the wasm client.
#[tracing::instrument]
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guild_channels(guild_id: String) -> SettingsResult<Vec<ChannelSummary>> {
    use crate::{
        auth::guild::require_guild_admin, oauth::discord::bot, settings::sort_text_channels,
    };

    require_guild_admin(&auth, &pool, &guild_id).await?;

    // the user's own oauth token can't list channels (scope is `identify
    // guilds`); this needs the bot's own token instead
    let channels = bot::get_guild_channels(&guild_id).await?;

    Ok(sort_text_channels(&channels))
}
