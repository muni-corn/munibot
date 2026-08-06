use std::collections::HashMap;

use dioxus::prelude::*;

use crate::{app::Route, pages::chat::sidebar::ConversationSidebar};

pub mod conversation;
pub mod sidebar;

/// A conversation's unsent draft text, keyed by conversation id, shared
/// across every `/chat/*` route via context.
///
/// Lives here rather than as a plain `use_signal` inside `ChatConversation`
/// itself: navigating between two conversations remounts that component
/// fresh (same route variant, different `conversation_id` prop, the same
/// way `LoggingSettingsPage` remounts per `guild_id`), which would reset a
/// signal local to it. `ChatLayout` only unmounts when leaving `/chat`
/// entirely, so a context it provides survives switching conversations.
#[derive(Clone, Copy)]
pub struct ChatDrafts(pub Signal<HashMap<i64, String>>);

/// Layout for every `/chat/*` route: the conversation sidebar beside
/// whichever `Chat`/`ChatConversation` route matched, in the outlet.
///
/// Not nested under `Dashboard`: that layout's sidebar is guild-scoped and
/// irrelevant to a person's own chat history, and this is munibot's own
/// page rather than a settings screen.
#[component]
pub fn ChatLayout() -> Element {
    use_context_provider(|| ChatDrafts(Signal::new(HashMap::new())));

    rsx! {
        div { class: "flex h-full flex-row",
            ConversationSidebar {}
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
