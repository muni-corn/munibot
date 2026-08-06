use dioxus::prelude::*;
use munibot_api::{
    chat::ChatEvent,
    server_fns::chat::{conversation::get_conversation_messages, stream::chat_stream},
};

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
/// The tool activity display arrives in a later commit;
/// `ChatEvent::ToolStarted`/ `ToolFinished` are received here already but not
/// yet shown.
#[component]
pub fn ChatConversation(conversation_id: i64) -> Element {
    let mut messages = use_resource(move || async move {
        get_conversation_messages(conversation_id, None, MESSAGE_PAGE_SIZE).await
    });
    // `Some("")` the instant a turn starts, growing as text deltas arrive, and
    // `None` once it's over -- see MessageList's own doc comment for why this
    // can't just be another entry in `messages` itself
    let mut live_reply = use_signal(|| None::<String>);
    let mut turn_error = use_signal(|| None::<String>);

    let on_sent = move |message_id: i64| {
        // the user's own message is already durably persisted by the time
        // send_message returns, so it's safe to reload the transcript right
        // away rather than waiting for the reply too
        messages.restart();
        turn_error.set(None);
        live_reply.set(Some(String::new()));

        spawn(async move {
            match chat_stream(message_id).await {
                Ok(mut stream) => {
                    while let Some(event) = stream.recv().await {
                        match event {
                            Ok(ChatEvent::TextDelta(text)) => {
                                if let Some(reply) = live_reply.write().as_mut() {
                                    reply.push_str(&text);
                                }
                            }
                            Ok(ChatEvent::Failed { message }) => turn_error.set(Some(message)),
                            // handled in a later commit: tool activity display, and
                            // the persona-driven ones (TurnStarted/IterationComplete/
                            // Thinking/Handoff) that nothing here shows yet
                            Ok(_) => {}
                            Err(error) => {
                                turn_error.set(Some(error.to_string()));
                                break;
                            }
                        }
                    }
                }
                Err(error) => turn_error.set(Some(error.to_string())),
            }

            live_reply.set(None);
            messages.restart();
        });
    };

    let content = match &*messages.read() {
        Some(Ok(loaded)) => rsx! {
            MessageList {
                messages: loaded.clone(),
                live_reply: live_reply.read().clone(),
            }
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
            if let Some(message) = &*turn_error.read() {
                div { class: "px-4 pb-2 text-sm text-error",
                    "munibot couldn't finish that :< {message}"
                }
            }
            Composer {
                conversation_id,
                disabled: live_reply.read().is_some(),
                on_sent,
            }
        }
    }
}
