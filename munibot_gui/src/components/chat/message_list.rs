use dioxus::prelude::*;
use munibot_api::chat::{ChatMessage, ChatRole};

use crate::components::chat::markdown::render_markdown;

/// Renders a conversation's messages as chat bubbles, oldest first.
#[component]
pub fn MessageList(messages: Vec<ChatMessage>) -> Element {
    rsx! {
        div { class: "flex flex-col p-4 gap-1",
            for message in messages {
                MessageBubble { key: "{message.id}", message }
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
