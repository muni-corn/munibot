use dioxus::{fullstack::AsStatusCode, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error returned by chat server functions.
///
/// Kept distinct from one generic failure, the same reasoning as
/// `crate::settings::SettingsError`: the chat page needs to show a different
/// message for each of these, not just print a string.
#[derive(Clone, Debug, Error, Serialize, Deserialize)]
pub enum ChatError {
    #[error("you need to sign in first")]
    NotSignedIn,

    /// `ai.enabled` is `false` in config, so no chat service was ever built.
    /// Distinct from every other failure so the chat page can say the chat
    /// is turned off instead of showing a scary generic error.
    #[error("chat isn't turned on right now")]
    AiDisabled,

    /// The conversation sent can't be answered, for a reason the person can
    /// act on. Carries the specific reason rather than folding into
    /// `ServerFnError`, since the chat page shows it verbatim.
    #[error("{0}")]
    InvalidConversation(String),

    /// Wraps a generic server function error so `ChatResult` propagates
    /// cleanly.
    #[error(transparent)]
    ServerFnError(#[from] ServerFnError),
}

impl AsStatusCode for ChatError {
    fn as_status_code(&self) -> StatusCode {
        match self {
            Self::NotSignedIn => StatusCode::UNAUTHORIZED,
            Self::AiDisabled => StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidConversation(_) => StatusCode::BAD_REQUEST,
            Self::ServerFnError(e) => e.as_status_code(),
        }
    }
}

pub type ChatResult<T> = Result<T, ChatError>;

#[cfg(feature = "server")]
impl From<munibot_ai::AiError> for ChatError {
    fn from(e: munibot_ai::AiError) -> Self {
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
    fn test_ai_disabled_is_service_unavailable() {
        assert_eq!(
            ChatError::AiDisabled.as_status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn test_attachment_rejected_is_bad_request_and_keeps_its_reason() {
        let err = ChatError::AttachmentRejected("that's not a supported image type".to_string());
        assert_eq!(err.as_status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(err.to_string(), "that's not a supported image type");
    }

    #[test]
    fn test_attachment_not_found_is_not_found() {
        assert_eq!(
            ChatError::AttachmentNotFound.as_status_code(),
            StatusCode::NOT_FOUND
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
