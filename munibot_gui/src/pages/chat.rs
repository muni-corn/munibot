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
pub fn ChatLayout() -> Element {
    use_context_provider(|| ChatDrafts(Signal::new(HashMap::new())));

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
