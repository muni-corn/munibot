use dioxus::prelude::*;

use crate::settings::{GuildLoggingSettings, SettingsResult};

/// Saves a guild's logging settings, returning what was actually saved.
///
/// Re-validates `channel_id` server-side against the guild's real channel
/// list even though the gui only ever offers channels from
/// `get_guild_logging_page` in its picker -- the client can't be trusted to
/// only ever send back a value it was shown.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn set_guild_logging_settings(
    guild_id: String,
    settings: GuildLoggingSettings,
) -> SettingsResult<GuildLoggingSettings> {
    use munibot_core::db::{models::GuildConfig, operations};

    use crate::{auth::guild::require_guild_admin, oauth::discord::bot};

    require_guild_admin(&auth, &pool, &guild_id).await?;

    if let Some(channel_id) = &settings.channel_id {
        let channels = bot::get_guild_channels(&guild_id).await?;
        let is_text_channel_in_this_guild = channels.iter().any(|channel| {
            &channel.id == channel_id
                && matches!(
                    channel.kind,
                    bot::CHANNEL_TYPE_TEXT | bot::CHANNEL_TYPE_ANNOUNCEMENT
                )
        });
        if !is_text_channel_in_this_guild {
            return Err(
                anyhow::anyhow!("'{channel_id}' isn't a text channel in this server").into(),
            );
        }
    }

    let guild_id_i64: i64 = guild_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid guild id '{guild_id}': {e}"))?;
    let channel_id_i64 = settings
        .channel_id
        .as_deref()
        .map(str::parse::<i64>)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid channel id: {e}"))?;

    operations::upsert_guild_config(&pool, GuildConfig {
        guild_id: guild_id_i64,
        logging_channel: channel_id_i64,
    })
    .await?;

    Ok(settings)
}
