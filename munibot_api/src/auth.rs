use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "server")]
pub mod guild;
#[cfg(feature = "server")]
pub mod server;

/// User data safe to send to and render on the client.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserData {
    pub display_name: String,
    pub avatar_url: Option<String>,
}

/// Error returned by auth-related server functions.
#[derive(Clone, Debug, Error, Serialize, Deserialize)]
pub enum AuthError {
    #[error("no auth session available")]
    NoAuthSession,

    /// Discord is rate limiting munibot's requests. `retry_after_secs` is a
    /// hint for how long the caller should wait before trying again.
    #[error("discord is rate limiting us; try again in a bit :<")]
    RateLimited { retry_after_secs: u64 },

    /// Wraps a generic server function error so `AuthResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for AuthError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NoAuthSession => StatusCode::UNAUTHORIZED,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::ServerFnError(e) => e.as_status_code(),
        }
    }
}

pub type AuthResult<T> = Result<T, AuthError>;

#[cfg(feature = "server")]
impl From<diesel::result::Error> for AuthError {
    fn from(e: diesel::result::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<anyhow::Error> for AuthError {
    fn from(e: anyhow::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<crate::oauth::discord::DiscordOAuthError> for AuthError {
    fn from(e: crate::oauth::discord::DiscordOAuthError) -> Self {
        match e {
            crate::oauth::discord::DiscordOAuthError::RateLimited { retry_after, .. } => {
                Self::RateLimited {
                    retry_after_secs: retry_after.as_secs(),
                }
            }
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
impl From<std::sync::Arc<crate::oauth::discord::DiscordOAuthError>> for AuthError {
    fn from(e: std::sync::Arc<crate::oauth::discord::DiscordOAuthError>) -> Self {
        match &*e {
            crate::oauth::discord::DiscordOAuthError::RateLimited { retry_after, .. } => {
                Self::RateLimited {
                    retry_after_secs: retry_after.as_secs(),
                }
            }
            other => Self::ServerFnError(ServerFnError::ServerError {
                message: other.to_string(),
                code: StatusCode::INTERNAL_SERVER_ERROR.into(),
                details: None,
            }),
        }
    }
}
