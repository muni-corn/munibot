use dioxus::prelude::*;
use munibot_api::{
    chat::MemoryEntry,
    server_fns::chat::memory::{
        forget_memory, get_memory_settings, list_memories, set_memory_opt_in, wipe_memories,
    },
};

use crate::components::{
    Spinner,
    settings::{SettingsRow, SettingsSection},
};

/// The visible half of the memory opt-in promise from phase 10: see, edit,
/// delete, and wipe everything munibot remembers, plus the opt-in toggle
/// itself. An opt-in you cannot audit is not really consent.
#[component]
pub fn Memory() -> Element {
    let mut settings = use_resource(get_memory_settings);
    let mut memories = use_resource(list_memories);
    let mut error = use_signal(|| None::<String>);
    let mut toggling = use_signal(|| false);

    let toggle_opt_in = move |_| {
        let currently_opted_in = settings
            .read()
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|settings| settings.opted_in)
            .unwrap_or(false);

        spawn(async move {
            toggling.set(true);
            match set_memory_opt_in(!currently_opted_in).await {
                Ok(_) => settings.restart(),
                Err(e) => error.set(Some(e.to_string())),
            }
            toggling.set(false);
        });
    };

    let wipe = move |_| {
        spawn(async move {
            match wipe_memories().await {
                Ok(()) => memories.restart(),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let content = match (&*settings.read(), &*memories.read()) {
        (Some(Ok(settings)), Some(Ok(entries))) => rsx! {
            SettingsSection {
                title: "memory".to_string(),
                description: Some(
                    "lets munibot save facts you ask him to remember, for later conversations."
                        .to_string(),
                ),
                SettingsRow {
                    label: "remember things about me".to_string(),
                    description: None,
                    input {
                        r#type: "checkbox",
                        class: "toggle toggle-primary",
                        checked: settings.opted_in,
                        disabled: *toggling.read(),
                        onchange: toggle_opt_in,
                    }
                }
            }
            SettingsSection { title: "what he remembers".to_string(), description: None,
                if entries.is_empty() {
                    p { class: "text-sm text-slate-400", "nothing recorded yet." }
                } else {
                    ul { class: "flex flex-col gap-2",
                        for memory in entries.iter() {
                            MemoryRow {
                                key: "{memory.key}",
                                memory: memory.clone(),
                                on_forgotten: move |_| memories.restart(),
                            }
                        }
                    }
                    button {
                        class: "btn btn-error btn-sm self-start",
                        onclick: wipe,
                        "wipe everything"
                    }
                }
            }
        },
        (Some(Err(e)), _) | (_, Some(Err(e))) => rsx! {
            div { class: "alert alert-error", "couldn't load your memory settings :< {e}" }
        },
        _ => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "memory ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "font-black text-2xl", "memory" }
            if let Some(message) = &*error.read() {
                div { class: "alert alert-error", {message.clone()} }
            }
            {content}
        }
    }
}

#[component]
fn MemoryRow(memory: MemoryEntry, on_forgotten: EventHandler<()>) -> Element {
    let key = memory.key.clone();
    let forget_click = move |_| {
        let key = key.clone();
        spawn(async move {
            let _ = forget_memory(key).await;
            on_forgotten.call(());
        });
    };

    rsx! {
        li { class: "flex items-center gap-4 justify-between rounded-box bg-slate-900/50 p-3",
            div { class: "flex flex-col",
                span { class: "font-semibold", {memory.key.clone()} }
                span { class: "text-sm text-slate-400", {memory.value.clone()} }
            }
            button { class: "btn btn-ghost btn-xs", onclick: forget_click,
                i { class: "ph-duotone ph-trash" }
            }
        }
    }
}
