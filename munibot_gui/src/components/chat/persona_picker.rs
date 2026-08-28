use dioxus::prelude::*;
use munibot_api::server_fns::chat::persona::list_personas;

/// The persona a new conversation defaults to, if the picker's own list
/// loads before someone changes the selection: munibot's default,
/// conversational persona is the only sensible thing to default to.
const DEFAULT_PERSONA_ID: &str = "companion";

/// A button that opens a small persona picker (a plain daisyUI `dropdown`,
/// css-only, no js needed to open or close it) and calls `on_picked` with
/// whichever persona id was chosen.
///
/// This replaces automatic persona routing for the web entirely: a visible,
/// user-controlled choice is better than an invisible personality switch,
/// especially for a companion.
#[component]
pub fn NewConversationButton(on_picked: EventHandler<String>) -> Element {
    let personas = use_resource(list_personas);
    let mut selected = use_signal(|| DEFAULT_PERSONA_ID.to_string());

    let content = match &*personas.read() {
        Some(Ok(list)) if !list.is_empty() => rsx! {
            select {
                class: "select w-full select-sm",
                value: "{selected}",
                onchange: move |event| selected.set(event.value()),
                for persona in list.iter() {
                    option { key: "{persona.id}", value: "{persona.id}",
                        {persona.display_name.clone()}
                    }
                }
            }
            button {
                class: "btn w-full btn-primary btn-sm",
                onclick: move |_| on_picked.call(selected.read().clone()),
                "start"
            }
        },
        Some(Ok(_)) => rsx! {
            span { class: "text-xs text-slate-400", "no personas are configured." }
        },
        Some(Err(_)) => rsx! {
            span { class: "text-xs text-error", "couldn't load personas." }
        },
        None => rsx! {
            span { class: "text-xs text-slate-400", "loading..." }
        },
    };

    rsx! {
        div { class: "dropdown dropdown-end",
            div {
                tabindex: 0,
                role: "button",
                class: "btn btn-square btn-ghost btn-sm",
                i { class: "ph-duotone ph-plus" }
            }
            div {
                tabindex: 0,
                class: "menu dropdown-content z-1 flex w-64 flex-col gap-2 rounded-box bg-slate-800 p-3 shadow-lg",
                {content}
            }
        }
    }
}
