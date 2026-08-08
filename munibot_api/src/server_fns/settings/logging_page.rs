use dioxus::prelude::*;

use crate::settings::{GuildLoggingPage, SettingsResult};

/// Returns everything the logging settings page needs to render: the
/// guild's postable channels and its current logging settings.
///
/// Supersedes calling `get_guild_channels` and `get_guild_logging_settings`
/// separately -- each of those calls `require_guild_admin` on its own,
/// which means loading the page used to make two near-simultaneous,
/// identical guild-admin checks (each a `get_current_user_guilds` call,
/// before the guild cache existed). This makes just one.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guild_logging_page(guild_id: String) -> SettingsResult<GuildLoggingPage> {
    use munibot_core::db::operations;

    use crate::{
        auth::guild::require_guild_admin, oauth::discord::bot, settings::sort_text_channels,
    };

    require_guild_admin(&auth, &pool, &guild_id).await?;

    let channels = bot::get_guild_channels(&guild_id).await?;
    let channels = sort_text_channels(&channels);

    let guild_id_i64: i64 = guild_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid guild id '{guild_id}': {e}"))?;

    let config = operations::get_guild_config(&pool, guild_id_i64).await?;

    let settings = crate::settings::GuildLoggingSettings {
        channel_id: config
            .and_then(|config| config.logging_channel)
            .map(|id| id.to_string()),
    };

    Ok(GuildLoggingPage { channels, settings })
}
