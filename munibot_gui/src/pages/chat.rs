use dioxus::prelude::*;
use munibot_api::{
    chat::{ChatEvent, ChatMessage},
    server_fns::chat::stream::chat_stream,
};

use crate::components::chat::{
    composer::Composer,
    message_list::MessageList,
    turn_failure::{TurnFailure, TurnFailureBanner},
};

/// A temporary conversation with munibot.
///
/// Nothing is saved: the conversation lives in this component's own state and
/// is sent back to the server in full with every message, so leaving or
/// reloading the page starts over.
#[component]
pub fn Chat() -> Element {
    let mut messages = use_signal(Vec::<ChatMessage>::new);
    // `Some("")` the instant a reply starts, growing as text deltas arrive,
    // and `None` once it's over -- kept out of `messages` until it finishes
    // so a reply that fails partway never becomes part of the conversation
    let mut live_reply = use_signal(|| None::<String>);
    let mut failure = use_signal(|| None::<TurnFailure>);

    // asks munibot to answer the conversation as it stands: used both for a
    // freshly sent message and for retrying one whose reply failed
    let mut run_turn = move || {
        failure.set(None);
        live_reply.set(Some(String::new()));
        let history = messages.read().clone();

        spawn(async move {
            match chat_stream(history).await {
                Ok(mut stream) => {
                    let mut finished = false;
                    while let Some(event) = stream.recv().await {
                        match event {
                            Ok(ChatEvent::TextDelta(text)) => {
                                if let Some(reply) = live_reply.write().as_mut() {
                                    reply.push_str(&text);
                                }
                            }
                            Ok(ChatEvent::Finished) => {
                                finished = true;
                                break;
                            }
                            Ok(ChatEvent::Failed(message)) => {
                                failure.set(Some(TurnFailure::Failed(message)));
                                break;
                            }
                            Err(error) => {
                                failure.set(Some(TurnFailure::Failed(error.to_string())));
                                break;
                            }
                        }
                    }

                    if finished {
                        let reply = live_reply.read().clone().unwrap_or_default();
                        if reply.trim().is_empty() {
                            failure.set(Some(TurnFailure::Failed(
                                "munibot didn't say anything back :<".to_string(),
                            )));
                        } else {
                            messages.write().push(ChatMessage::assistant(reply));
                        }
                    } else if failure.read().is_none() {
                        failure.set(Some(TurnFailure::Failed(
                            "the connection dropped before the reply finished :<".to_string(),
                        )));
                    }
                }
                Err(error) => failure.set(Some(TurnFailure::from_chat_error(&error))),
            }

            live_reply.set(None);
        });
    };

    let on_send = move |text: String| {
        messages.write().push(ChatMessage::user(text));
        run_turn();
    };

    let retry = move |_| run_turn();

    let is_replying = live_reply.read().is_some();

    rsx! {
        div { class: "flex h-full flex-col place-content-center items-center p-4 text-slate-300 gap-2",
            p { "pick a conversation from the sidebar, or start a new one." }
        }
    }
}
