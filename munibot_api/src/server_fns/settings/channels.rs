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
    use std::collections::HashMap;

    use crate::{auth::guild::require_guild_admin, oauth::discord::bot};

    require_guild_admin(&auth, &pool, &guild_id).await?;

    // the user's own oauth token can't list channels (scope is `identify
    // guilds`); this needs the bot's own token instead
    let bot_token =
        std::env::var("DISCORD_TOKEN").map_err(|_| anyhow::anyhow!("DISCORD_TOKEN isn't set"))?;

    let channels = bot::get_guild_channels(&bot_token, &guild_id).await?;

    // categories carry their own position among other categories; sort
    // uncategorized channels first (matching discord's own client), then by
    // each channel's position within its category. sorting by a channel's
    // raw position alone would interleave categories, since position only
    // orders channels within the same parent
    let category_positions: HashMap<&str, i32> = channels
        .iter()
        .filter(|channel| channel.kind == bot::CHANNEL_TYPE_CATEGORY)
        .map(|channel| (channel.id.as_str(), channel.position))
        .collect();
    let category_names: HashMap<&str, &str> = channels
        .iter()
        .filter(|channel| channel.kind == bot::CHANNEL_TYPE_CATEGORY)
        .map(|channel| (channel.id.as_str(), channel.name.as_str()))
        .collect();

    let mut text_channels: Vec<_> = channels
        .iter()
        .filter(|channel| {
            matches!(
                channel.kind,
                bot::CHANNEL_TYPE_TEXT | bot::CHANNEL_TYPE_ANNOUNCEMENT
            )
        })
        .collect();
    text_channels.sort_by_key(|channel| {
        let category_position = channel
            .parent_id
            .as_deref()
            .and_then(|id| category_positions.get(id))
            .copied()
            .unwrap_or(i32::MIN);
        (category_position, channel.position)
    });

    Ok(text_channels
        .into_iter()
        .map(|channel| ChannelSummary {
            id: channel.id.clone(),
            name: channel.name.clone(),
            category: channel
                .parent_id
                .as_deref()
                .and_then(|id| category_names.get(id))
                .map(|name| name.to_string()),
        })
        .collect())
}
