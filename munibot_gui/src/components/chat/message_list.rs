use dioxus::prelude::*;
use munibot_api::chat::{ChatMessage, ChatRole};

use crate::components::chat::markdown::render_markdown;

/// Renders a conversation's messages as chat bubbles, oldest first.
///
/// `live_reply` is the in-flight assistant reply's text so far, appended
/// after the finished messages: `Some("")` while waiting for the first token,
/// `Some(text)` as deltas arrive, and `None` when no reply is running. Kept
/// separate from `messages` since it isn't part of the conversation until it
/// finishes.
///
/// The scroll container is `flex-col-reverse`, which keeps the view pinned to
/// the newest message as the reply grows without any scripting.
#[component]
pub fn MessageList(messages: Vec<ChatMessage>, live_reply: Option<String>) -> Element {
    rsx! {
        div { class: "flex grow flex-col-reverse overflow-y-auto",
            div { class: "flex flex-col gap-1 p-4",
                for (index, message) in messages.into_iter().enumerate() {
                    MessageBubble { key: "{index}", message }
                }
                if let Some(text) = live_reply {
                    LiveReplyBubble { text }
                }
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
        div { class: "chat-start chat",
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
#[component]
fn MessageBubble(message: ChatMessage) -> Element {
    let (side, bubble_class) = match message.role {
        ChatRole::User => ("chat-end", "chat-bubble-primary"),
        ChatRole::Assistant => ("chat-start", ""),
    };

    rsx! {
        div { class: "chat {side}",
            div { class: "chat-bubble max-w-full {bubble_class}", {render_markdown(&message.content)} }
        }
    }
}
