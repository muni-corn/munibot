use dioxus::prelude::*;
use munibot_api::chat::{AttachmentSummary, ChatMessage, ChatRole};

use crate::components::chat::{
    delegation::{DelegationEntry, DelegationStrip},
    markdown::render_markdown,
    tool_activity::{ToolActivityEntry, ToolActivityStrip},
};

/// Renders a conversation's messages as chat bubbles, oldest first.
///
/// `live_reply` is the in-flight assistant turn's text so far, appended
/// after the loaded history: `Some("")` while waiting for the first token,
/// `Some(text)` as deltas arrive, and `None` when no turn is running. Kept
/// separate from `messages` itself rather than folded into it, since it has
/// no database row (and so no id) until the turn finishes and the
/// transcript is reloaded.
///
/// `tool_activity` and `delegations` are both shown directly above the live
/// reply, and only while `live_reply` is `Some` -- all three clear together
/// once the turn ends and the transcript reloads with the persisted reply,
/// rather than leaving either stranded below a bubble that has since
/// disappeared. `delegations` renders above `tool_activity`, since bringing
/// a specialist in is the more significant event of the two.
#[component]
pub fn MessageList(
    messages: Vec<ChatMessage>,
    live_reply: Option<String>,
    tool_activity: Vec<ToolActivityEntry>,
    delegations: Vec<DelegationEntry>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1 p-4",
            for message in messages {
                MessageBubble { key: "{message.id}", message }
            }
            if let Some(text) = live_reply {
                DelegationStrip { entries: delegations }
                ToolActivityStrip { entries: tool_activity }
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
            div { class: "chat-bubble max-w-full {bubble_class}",
                if !message.attachments.is_empty() {
                    div { class: "flex flex-wrap gap-2 pb-2",
                        for attachment in message.attachments.iter().cloned() {
                            AttachmentImage { key: "{attachment.id}", attachment }
                        }
                    }
                }
                {render_markdown(&message.content)}
            }
        }
    }
}

/// One attachment shown inline in a message's own bubble: a thumbnail
/// linking straight to `/attachments/{id}` (the plain axum route in
/// `munibot_gui::server::attachments`, not a server function -- a server
/// function's response is always JSON, so it can't be an `<img>`'s own
/// `src`) for a full-size view in a new tab, so scrolling back through a
/// conversation shows what was actually discussed rather than a
/// placeholder.
#[component]
fn AttachmentImage(attachment: AttachmentSummary) -> Element {
    let src = format!("/attachments/{}", attachment.id);
    rsx! {
        a { href: src.clone(), target: "_blank", rel: "noopener noreferrer",
            img {
                class: "max-h-48 rounded-box border border-slate-700 object-contain",
                src,
                alt: "an attached image",
            }
        }
    }
}
