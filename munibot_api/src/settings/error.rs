use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "server")]
use crate::oauth::discord::DiscordOAuthError;
#[cfg(feature = "server")]
use crate::oauth::discord::bot::DiscordBotError;

/// Error returned by settings server functions.
///
/// Kept distinct from every other failure a settings page can hit, rather
/// than collapsing them all into one generic "something went wrong" (or
/// worse, into "please sign in", which is what the dashboard's guild list
/// does today) -- a settings page needs to show a different message and
/// call to action for each of these.
#[derive(Clone, Debug, Error, Serialize, Deserialize)]
pub enum SettingsError {
    #[error("you need to sign in first")]
    NotSignedIn,

    #[error("you don't have permission to manage this server's settings")]
    NotGuildAdmin,

    #[error("munibot hasn't been invited to this server yet")]
    BotNotInGuild,

    /// Discord is rate limiting munibot's requests. `retry_after_secs` is a
    /// hint for how long the caller should wait before trying again.
    #[error("discord is rate limiting us; try again in a bit :<")]
    RateLimited { retry_after_secs: u64 },

    /// Wraps a generic server function error so `SettingsResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for SettingsError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NotSignedIn => StatusCode::UNAUTHORIZED,
            Self::NotGuildAdmin => StatusCode::FORBIDDEN,
            Self::BotNotInGuild => StatusCode::NOT_FOUND,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::ServerFnError(e) => e.as_status_code(),
        }
    }
}

pub type SettingsResult<T> = Result<T, SettingsError>;

#[cfg(feature = "server")]
impl From<diesel::result::Error> for SettingsError {
    fn from(e: diesel::result::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<anyhow::Error> for SettingsError {
    fn from(e: anyhow::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<DiscordOAuthError> for SettingsError {
    fn from(e: DiscordOAuthError) -> Self {
        match e {
            DiscordOAuthError::RateLimited { retry_after, .. } => Self::RateLimited {
                retry_after_secs: retry_after.as_secs(),
            },
            other => Self::ServerFnError(ServerFnError::ServerError {
                message: other.to_string(),
                code: StatusCode::INTERNAL_SERVER_ERROR.into(),
                details: None,
            }),
        }
    }
}

/// `try_get_with`'s coalesced error type -- multiple concurrent callers that
/// all miss the guild cache share one underlying request, and so also share
/// one `Arc`-wrapped error.
#[cfg(feature = "server")]
impl From<std::sync::Arc<DiscordOAuthError>> for SettingsError {
    fn from(e: std::sync::Arc<DiscordOAuthError>) -> Self {
        match &*e {
            DiscordOAuthError::RateLimited { retry_after, .. } => Self::RateLimited {
                retry_after_secs: retry_after.as_secs(),
            },
            other => Self::ServerFnError(ServerFnError::ServerError {
                message: other.to_string(),
                code: StatusCode::INTERNAL_SERVER_ERROR.into(),
                details: None,
            }),
        }
    }
}

#[cfg(feature = "server")]
impl From<DiscordBotError> for SettingsError {
    fn from(e: DiscordBotError) -> Self {
        match e {
            DiscordBotError::NotInGuild => Self::BotNotInGuild,
            DiscordBotError::RateLimited { retry_after, .. } => Self::RateLimited {
                retry_after_secs: retry_after.as_secs(),
            },
            other => Self::ServerFnError(ServerFnError::ServerError {
                message: other.to_string(),
                code: StatusCode::INTERNAL_SERVER_ERROR.into(),
                details: None,
            }),
        }
    }
}
