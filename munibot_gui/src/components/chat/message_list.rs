use dioxus::prelude::*;
use munibot_api::chat::{ChatMessage, ChatRole};

use crate::components::chat::markdown::render_markdown;

/// Renders a conversation's messages as chat bubbles, oldest first.
///
/// `live_reply` is the in-flight assistant turn's text so far, appended
/// after the loaded history: `Some("")` while waiting for the first token,
/// `Some(text)` as deltas arrive, and `None` when no turn is running. Kept
/// separate from `messages` itself rather than folded into it, since it has
/// no database row (and so no id) until the turn finishes and the
/// transcript is reloaded.
#[component]
pub fn MessageList(messages: Vec<ChatMessage>, live_reply: Option<String>) -> Element {
    rsx! {
        div { class: "flex flex-col p-4 gap-1",
            for message in messages {
                MessageBubble { key: "{message.id}", message }
            }
            if let Some(text) = live_reply {
                LiveReplyBubble { text }
            }
        }
    }
}

/// The in-flight assistant reply, styled the same as a finished
/// [`MessageBubble`] once text starts arriving, or a muted "thinking..."
/// placeholder before the first delta.
#[component]
fn LiveReplyBubble(text: String) -> Element {
    rsx! {
        div { class: "chat chat-start",
            div { class: "chat-bubble max-w-full",
                if text.is_empty() {
                    span { class: "opacity-60", "thinking..." }
                } else {
                    {render_markdown(&text)}
                }
            }
        }
    }
}

/// One message's bubble, styled and aligned by role.
///
/// System and tool-role messages are rare in a web transcript -- the system
/// prompt isn't stored as a message, and a stored tool result's content is
/// already dropped to empty by `ChatMessage::from_row` -- but are still
/// rendered, muted, rather than silently hidden: a missing bubble is more
/// confusing than an unstyled one.
#[component]
fn MessageBubble(message: ChatMessage) -> Element {
    let (side, bubble_class) = match message.role {
        ChatRole::User => ("chat-end", "chat-bubble-primary"),
        ChatRole::Assistant => ("chat-start", ""),
        ChatRole::System | ChatRole::Tool => {
            ("chat-start", "chat-bubble-neutral text-xs opacity-70")
        }
    };

    rsx! {
        div { class: "chat {side}",
            div { class: "chat-bubble max-w-full {bubble_class}", {render_markdown(&message.content)} }
        }
    }
}
