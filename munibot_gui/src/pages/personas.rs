use dioxus::prelude::*;
use munibot_api::{chat::PersonaSummary, server_fns::chat::persona::list_personas};

use crate::components::Spinner;

/// A readable listing of every configured persona: description, model, and
/// whether it remembers you. Doubles as user-facing documentation for the
/// specialist personas the chat page's own picker offers.
#[component]
pub fn Personas() -> Element {
    let personas = use_resource(list_personas);

    let content = match &*personas.read() {
        Some(Ok(personas)) if personas.is_empty() => rsx! {
            p { class: "text-sm text-slate-400", "no personas are configured." }
        },
        Some(Ok(personas)) => rsx! {
            div { class: "flex flex-col gap-4",
                for persona in personas.iter() {
                    PersonaCard { key: "{persona.id}", persona: persona.clone() }
                }
            }
        },
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load personas :< {e}" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "personas ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "font-black text-2xl", "personas" }
            {content}
        }
    }
}

#[component]
fn PersonaCard(persona: PersonaSummary) -> Element {
    rsx! {
        div { class: "flex flex-col p-4 gap-2 rounded-box bg-slate-900/50",
            div { class: "flex items-center justify-between",
                span { class: "font-black text-lg", {persona.display_name.clone()} }
                span { class: "font-mono text-xs text-slate-400", {persona.model.clone()} }
            }
            p { class: "text-slate-300 text-sm", {persona.description.clone()} }
            if persona.remembers_you {
                span { class: "badge badge-primary badge-sm self-start",
                    "remembers you across conversations"
                }
            }
        }
    }
}
