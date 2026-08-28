use dioxus::prelude::*;
use munibot_api::chat::{ChatError, ChatFailureKind};

/// Enough structure about a failed turn to react appropriately in the
/// chat page, unifying the two places a failure can come from:
/// [`ChatError`] (the stream failed to even start) and
/// [`ChatFailureKind`] (the turn started, but the model's reply itself
/// failed).
#[derive(Clone, Debug, PartialEq)]
pub enum TurnFailure {
    /// The session has expired or was signed out from another tab.
    NotSignedIn,
    /// `ai.enabled` is `false` in config.
    AiDisabled,
    /// A configured budget was hit. Retrying immediately would just hit it
    /// again, so no retry is offered.
    BudgetExceeded(String),
    /// A transient, provider-side problem - worth offering a retry for.
    Transient(String),
    /// Anything else. Not obviously worth a blind retry.
    Other(String),
}

impl TurnFailure {
    /// Maps a failure from starting the stream itself (before a turn ever
    /// began), as opposed to [`Self::from_event`], which maps a failure the
    /// turn reported after starting.
    pub fn from_chat_error(error: &ChatError) -> Self {
        match error {
            ChatError::NotSignedIn => Self::NotSignedIn,
            ChatError::AiDisabled => Self::AiDisabled,
            other => Self::Other(other.to_string()),
        }
    }

    /// Maps a `ChatEvent::Failed`'s own kind and message.
    pub fn from_event(kind: ChatFailureKind, message: String) -> Self {
        match kind {
            ChatFailureKind::BudgetExceeded => Self::BudgetExceeded(message),
            ChatFailureKind::Transient => Self::Transient(message),
            ChatFailureKind::Other => Self::Other(message),
        }
    }

    /// Maps a transport-level failure reading the stream itself (a dropped
    /// connection, a deserialization error) - always worth a retry, the
    /// same as a transient provider outage.
    pub fn from_transport_error(message: String) -> Self {
        Self::Transient(message)
    }
}

/// An inline banner for a failed turn, matched structurally rather than
/// showing one generic message for everything: signed-out prompts a
/// sign-in, a budget refusal explains itself kindly, and a transient
/// provider outage offers a retry that re-asks munibot to answer the same,
/// already-persisted message rather than sending a new one.
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
            div { class: "mx-4 mb-2 alert text-sm alert-warning",
                "the companion isn't turned on right now :<"
            }
        },
        TurnFailure::BudgetExceeded(message) => rsx! {
            div { class: "mx-4 mb-2 alert text-sm alert-warning", {message} }
        },
        TurnFailure::Transient(message) => rsx! {
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
            div { class: "mx-4 mb-2 alert text-sm alert-error", {message} }
        },
    }
}
