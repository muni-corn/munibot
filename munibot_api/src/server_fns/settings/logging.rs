use dioxus::prelude::*;

use crate::settings::{GuildLoggingSettings, SettingsResult};

/// Returns a guild's logging settings.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guild_logging_settings(guild_id: String) -> SettingsResult<GuildLoggingSettings> {
    use munibot_core::db::operations;

    use crate::auth::guild::require_guild_admin;

    require_guild_admin(&auth, &pool, &guild_id).await?;

    let guild_id_i64: i64 = guild_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid guild id '{guild_id}': {e}"))?;

    let config = operations::get_guild_config(&pool, guild_id_i64).await?;

    Ok(GuildLoggingSettings {
        channel_id: config
            .and_then(|config| config.logging_channel)
            .map(|id| id.to_string()),
    })
}
