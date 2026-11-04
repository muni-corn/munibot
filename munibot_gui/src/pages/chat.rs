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

    rsx! {
        div { class: "flex h-full flex-row",
            div { class: "p-4 text-slate-300 w-64 border-e border-slate-800", "conversations" }
            div { class: "grow bg-slate-950/50 sm:rounded-ss-3xl", Outlet::<Route> {} }
        }
    }
}

/// Content shown in `ChatLayout`'s outlet at the bare `/chat` path, before a
/// conversation has been picked from the sidebar or started.
#[component]
pub fn Chat() -> Element {
    rsx! {
        div { class: "flex h-full flex-col place-content-center items-center p-4 text-slate-300 gap-2",
            p { "pick a conversation from the sidebar, or start a new one." }
        }
    }
}
