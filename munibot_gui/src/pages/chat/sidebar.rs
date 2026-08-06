use dioxus::{html::keyboard_types::Key, prelude::*};
use munibot_api::{
    chat::ConversationSummary,
    server_fns::chat::conversation::{
        archive_conversation, create_conversation, list_conversations, rename_conversation,
    },
};

use crate::{
    app::Route,
    components::{Spinner, chat::persona_picker::NewConversationButton},
};

/// The list of the signed-in user's conversations, newest first, with
/// new/rename/archive.
///
/// Owns its own `list_conversations` resource rather than receiving one
/// from `ChatLayout`: this is the only place that reads it, so there is
/// nothing for lifting it up to the layout to simplify.
#[component]
pub fn ConversationSidebar() -> Element {
    let mut conversations = use_resource(list_conversations);
    let mut error = use_signal(|| None::<String>);
    let navigator = use_navigator();

    let start_new = move |persona_id: String| {
        spawn(async move {
            match create_conversation(persona_id).await {
                Ok(created) => {
                    conversations.restart();
                    navigator.push(Route::ChatConversation {
                        conversation_id: created.id,
                    });
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let content = match &*conversations.read() {
        Some(Ok(entries)) if entries.is_empty() => rsx! {
            div { class: "flex grow flex-col place-content-center items-center p-4 gap-2 text-center text-slate-400",
                p { "no conversations yet." }
                NewConversationButton { on_picked: start_new }
            }
        },
        Some(Ok(entries)) => rsx! {
            ul { class: "menu w-full flex-1 gap-1 overflow-y-auto p-0",
                for entry in entries.iter() {
                    ConversationRow {
                        key: "{entry.id}",
                        entry: entry.clone(),
                        on_changed: move |_| conversations.restart(),
                        on_error: move |message| error.set(Some(message)),
                    }
                }
            }
        },
        Some(Err(e)) => rsx! {
            div { class: "p-4 text-sm text-error", "couldn't load your conversations :< {e}" }
        },
        None => rsx! {
            div { class: "flex grow place-content-center p-4", Spinner {} }
        },
    };

    rsx! {
        div { class: "flex h-full flex-col p-4 w-64 gap-2 border-e border-slate-800",
            div { class: "flex items-center justify-between",
                span { class: "font-black", "conversations" }
                NewConversationButton { on_picked: start_new }
            }
            if let Some(message) = &*error.read() {
                div { class: "alert alert-error alert-sm text-xs", {message.as_str()} }
            }
            {content}
        }
    }
}

/// One row in the sidebar: a link to the conversation, plus rename and
/// archive actions.
///
/// Renaming edits in place rather than through a modal or a `prompt()` --
/// no such component exists yet in the design system, and a modal is more
/// machinery than renaming a single line of text needs.
#[component]
fn ConversationRow(
    entry: ConversationSummary,
    on_changed: EventHandler<()>,
    on_error: EventHandler<String>,
) -> Element {
    let mut editing = use_signal(|| false);
    let mut draft_title = use_signal(String::new);

    let entry_id = entry.id;
    let entry_title = entry.title.clone();

    let start_editing = move |event: Event<MouseData>| {
        event.stop_propagation();
        draft_title.set(entry_title.clone().unwrap_or_default());
        editing.set(true);
    };

    let save = move || {
        let title = draft_title.read().clone();
        spawn(async move {
            match rename_conversation(entry_id, title).await {
                Ok(_) => {
                    editing.set(false);
                    on_changed.call(());
                }
                Err(e) => on_error.call(e.to_string()),
            }
        });
    };

    let archive = move |event: Event<MouseData>| {
        event.stop_propagation();
        spawn(async move {
            match archive_conversation(entry_id).await {
                Ok(()) => on_changed.call(()),
                Err(e) => on_error.call(e.to_string()),
            }
        });
    };

    if *editing.read() {
        return rsx! {
            li {
                input {
                    class: "input input-sm w-full",
                    value: "{draft_title}",
                    autofocus: true,
                    oninput: move |event| draft_title.set(event.value()),
                    onkeydown: move |event| match event.key() {
                        Key::Enter => save(),
                        Key::Escape => editing.set(false),
                        _ => {}
                    },
                    onblur: move |_| save(),
                }
            }
        };
    }

    let display_title = entry
        .title
        .clone()
        .unwrap_or_else(|| "new conversation".to_string());

    rsx! {
        li {
            Link {
                to: Route::ChatConversation {
                    conversation_id: entry.id,
                },
                class: "flex items-center justify-between gap-2",
                span { class: "truncate", {display_title} }
                span { class: "flex gap-1",
                    button { class: "btn btn-ghost btn-xs", onclick: start_editing,
                        i { class: "ph-duotone ph-pencil-simple" }
                    }
                    button { class: "btn btn-ghost btn-xs", onclick: archive,
                        i { class: "ph-duotone ph-archive" }
                    }
                }
            }
        }
    }
}
