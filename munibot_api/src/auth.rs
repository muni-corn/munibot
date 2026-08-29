use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "server")]
pub mod guild;
pub mod linked_account;
#[cfg(feature = "server")]
pub mod operator;
#[cfg(feature = "server")]
pub mod server;

pub use linked_account::LinkedAccountSummary;

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

    /// Attempted to link a provider account already linked to a
    /// *different* munibot user.
    #[error("that account is already linked to a different munibot user")]
    AlreadyLinkedElsewhere,

    /// Attempted to unlink a user's only remaining sign-in method.
    #[error("you can't unlink your last remaining sign-in method")]
    LastRemainingAccount,

    /// Wraps a generic server function error so `AuthResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for AuthError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NoAuthSession => StatusCode::UNAUTHORIZED,
            Self::AlreadyLinkedElsewhere | Self::LastRemainingAccount => StatusCode::CONFLICT,
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
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}
