//! Discord OAuth2 client: authorization-code exchange, and the REST calls
//! made with the resulting user access token.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod bot;

const API_BASE: &str = "https://discord.com/api/v10";

/// Scopes requested during the authorize step: `identify` for basic account
/// info, `guilds` for listing the servers the user is in.
const SCOPES: &str = "identify guilds";

/// Error talking to discord's oauth2 or REST endpoints.
#[derive(Debug, Error)]
pub enum DiscordOAuthError {
    #[error("request to discord failed :< {0}")]
    Request(#[from] reqwest::Error),

    #[error("discord returned an error: {error} ({error_description:?})")]
    Discord {
        error: String,
        error_description: Option<String>,
    },
}

/// The redirect URI munibot registers with discord for the oauth2 callback.
/// `base_url` is munibot's own public base url (e.g. `http://localhost:8080`
/// in dev, or the real domain in production).
pub fn redirect_uri(base_url: &str) -> String {
    format!("{base_url}/auth/discord/callback")
}

/// Builds the URL to redirect a user to for discord's consent screen.
///
/// `state` is an opaque, unguessable value the caller generated and
/// stored server-side (see `oauth::routes`'s own CSRF state helpers) -
/// discord returns it verbatim on the callback, letting that handler
/// confirm the request actually originated from a real authorize
/// redirect this server issued, not a forged callback.
pub fn authorize_url(base_url: &str, client_id: &str, state: &str) -> String {
    let mut url =
        reqwest::Url::parse("https://discord.com/oauth2/authorize").expect("static url is valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri(base_url))
        .append_pair("scope", SCOPES)
        .append_pair("state", state);
    url.into()
}

/// A successful authorization-code exchange.
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds from now until the access token expires.
    pub expires_in: i64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenResponse {
    Success {
        access_token: String,
        refresh_token: String,
        expires_in: i64,
    },
    Error {
        error: String,
        error_description: Option<String>,
    },
}

/// Exchanges an authorization code (from the callback's `?code=` query
/// parameter) for an access token.
pub async fn exchange_code(
    code: &str,
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<Token, DiscordOAuthError> {
    let response = reqwest::Client::new()
        .post(format!("{API_BASE}/oauth2/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri(base_url)),
        ])
        .basic_auth(client_id, Some(client_secret))
        .send()
        .await?
        .json::<TokenResponse>()
        .await?;

    match response {
        TokenResponse::Success {
            access_token,
            refresh_token,
            expires_in,
        } => Ok(Token {
            access_token,
            refresh_token,
            expires_in,
        }),
        TokenResponse::Error {
            error,
            error_description,
        } => Err(DiscordOAuthError::Discord {
            error,
            error_description,
        }),
    }
}

/// The subset of discord's user object munibot cares about.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

impl DiscordUser {
    /// The name to show for this user: their global display name if set,
    /// falling back to their username.
    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.username)
    }

    /// This user's full avatar URL, if they have one set.
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar
            .as_ref()
            .map(|hash| format!("https://cdn.discordapp.com/avatars/{}/{hash}.png", self.id))
    }
}

/// Fetches the identity of the user who owns `access_token`.
pub async fn get_current_user(access_token: &str) -> Result<DiscordUser, DiscordOAuthError> {
    Ok(reqwest::Client::new()
        .get(format!("{API_BASE}/users/@me"))
        .bearer_auth(access_token)
        .send()
        .await?
        .json::<DiscordUser>()
        .await?)
}

/// The `MANAGE_GUILD` permission bit, per discord's permissions bitfield.
const MANAGE_GUILD: u64 = 0x20;

/// A guild (server) the user is a member of, as returned by
/// `/users/@me/guilds`.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordGuild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub owner: bool,
    /// Stringified permission bitfield for this user in this guild.
    pub permissions: String,
}

impl DiscordGuild {
    /// Whether this user owns or can manage (i.e. administrate) this guild.
    pub fn is_administered_by_user(&self) -> bool {
        self.owner
            || self
                .permissions
                .parse::<u64>()
                .is_ok_and(|bits| bits & MANAGE_GUILD != 0)
    }

    /// This guild's full icon URL, if it has one set.
    pub fn icon_url(&self) -> Option<String> {
        self.icon
            .as_ref()
            .map(|hash| format!("https://cdn.discordapp.com/icons/{}/{hash}.png", self.id))
    }
}

/// Fetches the guilds the user who owns `access_token` is a member of.
pub async fn get_current_user_guilds(
    access_token: &str,
) -> Result<Vec<DiscordGuild>, DiscordOAuthError> {
    Ok(reqwest::Client::new()
        .get(format!("{API_BASE}/users/@me/guilds"))
        .bearer_auth(access_token)
        .send()
        .await?
        .json::<Vec<DiscordGuild>>()
        .await?)
}
