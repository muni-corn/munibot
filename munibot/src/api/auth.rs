use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

    /// Wraps a generic server function error so `AuthResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for AuthError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NoAuthSession => StatusCode::UNAUTHORIZED,
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
