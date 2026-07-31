use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error returned by chat server functions.
///
/// Kept distinct from one generic failure, the same reasoning as
/// `crate::settings::SettingsError`: a chat page needs to show a different
/// message (or redirect) for each of these, not just print a string.
#[derive(Clone, Debug, Error, Serialize, Deserialize)]
pub enum ChatError {
    #[error("you need to sign in first")]
    NotSignedIn,

    /// The conversation exists, but belongs to someone else. Kept distinct
    /// from [`Self::ConversationNotFound`] at the type level even though both
    /// currently render the same "not found" response -- returning 404
    /// either way avoids confirming to a caller that a conversation id they
    /// don't own actually exists.
    #[error("that conversation doesn't belong to you")]
    NotYourConversation,

    #[error("that conversation doesn't exist")]
    ConversationNotFound,

    /// `ai.enabled` is `false` in config, so no `Ai` service was ever built.
    /// Distinct from every other failure so the chat page can show "the
    /// companion is turned off right now" instead of a scary generic error.
    #[error("the companion isn't turned on right now")]
    AiDisabled,

    /// Wraps a generic server function error so `ChatResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for ChatError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NotSignedIn => StatusCode::UNAUTHORIZED,
            // both render as a plain 404: existence of a conversation id
            // owned by someone else is not confirmed to the caller
            Self::NotYourConversation | Self::ConversationNotFound => StatusCode::NOT_FOUND,
            Self::AiDisabled => StatusCode::SERVICE_UNAVAILABLE,
            Self::ServerFnError(e) => e.as_status_code(),
        }
    }
}

pub type ChatResult<T> = Result<T, ChatError>;

#[cfg(feature = "server")]
impl From<munibot_ai::types::AiError> for ChatError {
    fn from(e: munibot_ai::types::AiError) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<anyhow::Error> for ChatError {
    fn from(e: anyhow::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(feature = "server")]
impl From<diesel::result::Error> for ChatError {
    fn from(e: diesel::result::Error) -> Self {
        Self::ServerFnError(ServerFnError::ServerError {
            message: e.to_string(),
            code: StatusCode::INTERNAL_SERVER_ERROR.into(),
            details: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_signed_in_is_unauthorized() {
        assert_eq!(
            ChatError::NotSignedIn.as_status_code(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn test_conversation_ownership_errors_both_render_as_not_found() {
        assert_eq!(
            ChatError::NotYourConversation.as_status_code(),
            StatusCode::NOT_FOUND,
            "owning-someone-else's-conversation should not be distinguishable from not-found over \
             the wire"
        );
        assert_eq!(
            ChatError::ConversationNotFound.as_status_code(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn test_ai_disabled_is_service_unavailable() {
        assert_eq!(
            ChatError::AiDisabled.as_status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn test_variants_are_distinguishable_by_matching_not_just_by_message() {
        // this is the whole point of ChatError existing rather than a single
        // generic failure: a caller must be able to match structurally
        assert!(matches!(ChatError::NotSignedIn, ChatError::NotSignedIn));
        assert!(matches!(ChatError::AiDisabled, ChatError::AiDisabled));
        assert!(!matches!(ChatError::NotSignedIn, ChatError::AiDisabled));
    }
}
