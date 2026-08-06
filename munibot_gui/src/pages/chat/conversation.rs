use dioxus::prelude::*;
use munibot_api::server_fns::chat::conversation::get_conversation_messages;

use crate::components::{
    Spinner,
    chat::{composer::Composer, message_list::MessageList},
};

/// How many of a conversation's most recent messages load up front.
///
/// Loading older history a page at a time (the cursor
/// `get_conversation_messages` already accepts for it) isn't part of this phase
/// yet -- a companion conversation is rarely long enough for one page to
/// matter, and it's straightforward to add once it does.
const MESSAGE_PAGE_SIZE: i64 = 100;

/// One conversation's transcript and composer.
///
/// The streaming reply and tool activity display arrive in their own later
/// commits: sending a message persists it and reloads the transcript, but
/// nothing yet actually asks munibot to answer it.
#[component]
pub fn ChatConversation(conversation_id: i64) -> Element {
    let mut messages = use_resource(move || async move {
        get_conversation_messages(conversation_id, None, MESSAGE_PAGE_SIZE).await
    });

    let content = match &*messages.read() {
        Some(Ok(messages)) => rsx! {
            MessageList { messages: messages.clone() }
        },
        Some(Err(e)) => rsx! {
            div { class: "p-4 text-sm text-error", "couldn't load this conversation :< {e}" }
        },
        None => rsx! {
            div { class: "flex h-full place-content-center p-4", Spinner {} }
        },
    };

    rsx! {
        document::Title { "chat ~ munibot" }
        div { class: "flex h-full flex-col",
            div { class: "grow overflow-y-auto", {content} }
            Composer {
                conversation_id,
                disabled: false,
                on_sent: move |_message_id| messages.restart(),
            }
        }
    }
}
