use dioxus::{fullstack::RequestError, prelude::*};
use munibot_api::chat::ChatError;

/// Why a reply didn't arrive, with enough structure to react appropriately
/// in the chat page rather than showing one generic message for everything.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnFailure {
    /// The session has expired or was signed out from another tab.
    NotSignedIn,
    /// `ai.enabled` is `false` in config.
    AiDisabled,
    /// Anything else, with the message to show.
    Failed(String),
}

impl TurnFailure {
    /// Maps a failure from starting the reply stream itself, before any
    /// reply began.
    ///
    /// A refusal arrives either as a typed [`ChatError`] or, because the
    /// stream's status is checked before its body could carry one, as a bare
    /// http status -- both mean the same thing here.
    pub fn from_chat_error(error: &ChatError) -> Self {
        match error {
            ChatError::NotSignedIn => Self::NotSignedIn,
            ChatError::AiDisabled => Self::AiDisabled,
            ChatError::ServerFnError(ServerFnError::Request(RequestError::Status(_, 401))) => {
                Self::NotSignedIn
            }
            ChatError::ServerFnError(ServerFnError::Request(RequestError::Status(_, 503))) => {
                Self::AiDisabled
            }
            other => Self::Failed(other.to_string()),
        }
    }
}

/// An inline banner for a failed reply: signed-out prompts a sign-in, a
/// disabled chat explains itself, and anything else offers a retry that asks
/// munibot to answer the same conversation again.
#[component]
pub fn TurnFailureBanner(failure: TurnFailure, on_retry: EventHandler<()>) -> Element {
    match failure {
        TurnFailure::NotSignedIn => rsx! {
            div { class: "mx-4 mb-2 alert text-sm alert-warning",
                span { "you've been signed out. " }
                a { class: "link", href: "/auth/discord/authorize", "sign in again" }
                span { " to keep chatting." }
            }
        },
        TurnFailure::AiDisabled => rsx! {
            div { class: "mx-4 mb-2 alert text-sm alert-warning", "chat isn't turned on right now :<" }
        },
        TurnFailure::Failed(message) => rsx! {
            div { class: "mx-4 mb-2 alert flex items-center justify-between text-sm alert-error",
                span { {message} }
                button {
                    class: "btn btn-ghost btn-xs",
                    onclick: move |_| on_retry.call(()),
                    "retry"
                }
            }
        },
        TurnFailure::Other(message) => rsx! {
            div { class: "alert alert-error mx-4 mb-2 text-sm", {message} }
        },
    }
}
