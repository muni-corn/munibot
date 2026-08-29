use dioxus::prelude::*;

use crate::settings::{GuildAiSettings, SettingsResult};

/// Returns a guild's ai settings.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_guild_ai_settings(guild_id: String) -> SettingsResult<GuildAiSettings> {
    use munibot_core::db::{
        models::DEFAULT_AI_CHANNEL_MODE,
        operations::{self, ai as ai_ops},
    };

    use crate::auth::guild::require_guild_admin;

    require_guild_admin(&auth, &pool, &guild_id).await?;

    let guild_id_i64: i64 = guild_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid guild id '{guild_id}': {e}"))?;

    let config = operations::get_guild_config(&pool, guild_id_i64).await?;
    let allowlist = ai_ops::list_ai_channel_allowlist(&pool, guild_id_i64).await?;

    let (enabled, default_persona, channel_mode) = match config {
        Some(config) => (
            config.ai_enabled,
            config.ai_default_persona,
            config.ai_channel_mode,
        ),
        None => (false, None, DEFAULT_AI_CHANNEL_MODE.to_string()),
    };

    Ok(GuildAiSettings {
        enabled,
        default_persona,
        channel_mode,
        channel_allowlist: allowlist.into_iter().map(|id| id.to_string()).collect(),
    })
}

/// Saves a guild's ai settings, returning what was actually saved.
///
/// Re-validates `default_persona` and every entry in `channel_allowlist`
/// server-side, the same reasoning `set_guild_logging_settings` documents
/// for its own `channel_id` -- the client can't be trusted to only ever
/// send back a persona id or a channel it was actually shown.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Ai>>>,
)]
pub async fn set_guild_ai_settings(
    guild_id: String,
    settings: GuildAiSettings,
) -> SettingsResult<GuildAiSettings> {
    use munibot_ai::persona::PersonaId;
    use munibot_core::db::operations::{self, ai as ai_ops};

    use crate::{
        auth::guild::require_guild_admin,
        oauth::discord::bot,
        settings::{CHANNEL_MODE_ALL, CHANNEL_MODE_ALLOWLIST},
    };

    require_guild_admin(&auth, &pool, &guild_id).await?;

    if settings.channel_mode != CHANNEL_MODE_ALL && settings.channel_mode != CHANNEL_MODE_ALLOWLIST
    {
        return Err(
            anyhow::anyhow!("'{}' isn't a valid channel mode", settings.channel_mode).into(),
        );
    }

    if let Some(persona_id) = &settings.default_persona {
        let ai_service =
            ai.0.as_ref()
                .ok_or_else(|| anyhow::anyhow!("ai isn't enabled on this server"))?;
        if ai_service
            .persona(&PersonaId::new(persona_id.clone()))
            .is_none()
        {
            return Err(anyhow::anyhow!("'{persona_id}' isn't a known persona").into());
        }
    }

    let guild_id_i64: i64 = guild_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid guild id '{guild_id}': {e}"))?;

    let mut channel_ids = Vec::with_capacity(settings.channel_allowlist.len());
    if !settings.channel_allowlist.is_empty() {
        let bot_token = std::env::var("DISCORD_TOKEN")
            .map_err(|_| anyhow::anyhow!("DISCORD_TOKEN isn't set"))?;
        let channels = bot::get_guild_channels(&bot_token, &guild_id).await?;

        for channel_id in &settings.channel_allowlist {
            if !channels.iter().any(|channel| &channel.id == channel_id) {
                return Err(
                    anyhow::anyhow!("'{channel_id}' isn't a channel in this server").into(),
                );
            }
            let channel_id_i64: i64 = channel_id
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid channel id: {e}"))?;
            channel_ids.push(channel_id_i64);
        }
    }

    operations::set_guild_ai_settings(
        &pool,
        guild_id_i64,
        settings.enabled,
        settings.default_persona.clone(),
        settings.channel_mode.clone(),
    )
    .await?;
    ai_ops::set_ai_channel_allowlist(&pool, guild_id_i64, &channel_ids).await?;

    Ok(settings)
}
