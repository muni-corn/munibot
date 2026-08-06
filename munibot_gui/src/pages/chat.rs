use dioxus::prelude::*;

use crate::app::Route;

pub mod conversation;

/// Layout for every `/chat/*` route: the conversation sidebar beside
/// whichever `Chat`/`ChatConversation` route matched, in the outlet.
///
/// Not nested under `Dashboard`: that layout's sidebar is guild-scoped and
/// irrelevant to a person's own chat history, and this is munibot's own
/// page rather than a settings screen.
///
/// The sidebar itself is a placeholder here -- the real conversation list,
/// from `list_conversations`, arrives in a later commit.
#[component]
pub fn ChatLayout() -> Element {
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
