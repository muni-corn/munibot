//! REST calls made with munibot's own bot token, as opposed to a signed-in
//! user's oauth token (see the parent module for those).
//!
//! A user's oauth scope is `identify guilds` (see `SCOPES` in the parent
//! module), which does not permit listing a guild's channels -- that needs
//! the bot's own token instead.

use serde::Deserialize;
use thiserror::Error;

/// Errors from a request made with the bot's own token.
#[derive(Debug, Error)]
pub enum DiscordBotError {
    #[error("request to discord failed :< {0}")]
    Request(#[from] reqwest::Error),

    /// Discord returned 403 or 404 for a guild-scoped request. Both mean
    /// the same thing from munibot's side: it isn't a member of that
    /// guild (a 403 can also mean a permissions problem, but the bot
    /// requests no permissions that would cause that here).
    #[error("munibot isn't a member of this server")]
    NotInGuild,

    #[error("discord returned an unexpected response: {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
}

/// The subset of a discord channel's REST representation munibot cares
/// about, as returned by `GET /guilds/{id}/channels`.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordChannel {
    pub id: String,
    /// Discord's numeric channel type -- 0 is text, 4 is category, 5 is
    /// announcement, etc.
    #[serde(rename = "type")]
    pub kind: u8,
    pub name: String,
    pub parent_id: Option<String>,
    pub position: i32,
}

/// Discord's numeric channel type for a plain text channel.
pub const CHANNEL_TYPE_TEXT: u8 = 0;
/// Discord's numeric channel type for a category (used to group other
/// channels; never itself a postable channel).
pub const CHANNEL_TYPE_CATEGORY: u8 = 4;
/// Discord's numeric channel type for an announcement channel.
pub const CHANNEL_TYPE_ANNOUNCEMENT: u8 = 5;

/// Fetches every channel in a guild, using the bot's own token.
pub async fn get_guild_channels(
    bot_token: &str,
    guild_id: &str,
) -> Result<Vec<DiscordChannel>, DiscordBotError> {
    let response = reqwest::Client::new()
        .get(format!("{}/guilds/{guild_id}/channels", super::API_BASE))
        .header("Authorization", format!("Bot {bot_token}"))
        .send()
        .await?;

    match response.status() {
        status if status.is_success() => Ok(response.json::<Vec<DiscordChannel>>().await?),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND => {
            Err(DiscordBotError::NotInGuild)
        }
        status => Err(DiscordBotError::UnexpectedStatus { status }),
    }
}
