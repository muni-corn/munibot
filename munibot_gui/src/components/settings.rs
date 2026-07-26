use dioxus::prelude::*;

use crate::components::Spinner;

/// A titled group of related settings within a settings page, e.g.
/// "logging" or "autodelete".
#[component]
pub fn SettingsSection(title: String, description: Option<String>, children: Element) -> Element {
    rsx! {
        section { class: "flex flex-col gap-4 rounded-box bg-slate-900/50 p-6",
            div { class: "flex flex-col gap-1",
                h3 { class: "font-black text-xl", {title} }
                if let Some(description) = description {
                    p { class: "text-sm text-slate-400", {description} }
                }
            }
            div { class: "flex flex-col gap-4", {children} }
        }
    }
}

/// A single labeled setting within a `SettingsSection`: a label and
/// description on one side, and the actual control (a select, toggle,
/// input, ...) on the other.
#[component]
pub fn SettingsRow(label: String, description: Option<String>, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-col sm:flex-row sm:items-center gap-2 sm:justify-between",
            div { class: "flex flex-col",
                span { class: "font-semibold", {label} }
                if let Some(description) = description {
                    span { class: "text-sm text-slate-400", {description} }
                }
            }
            div { class: "sm:min-w-64", {children} }
        }
    }
}

/// A sticky bar offering to save or discard unsaved changes to a settings
/// form. Renders nothing while `dirty` is `false`.
#[component]
pub fn SaveBar(
    dirty: bool,
    saving: bool,
    on_save: EventHandler<()>,
    on_discard: EventHandler<()>,
) -> Element {
    if !dirty {
        return rsx! {};
    }

    rsx! {
        div { class: "flex items-center gap-4 bg-slate-800 p-4 sticky bottom-0 justify-between rounded-box shadow-lg",
            span { class: "text-slate-300 text-sm", "you have unsaved changes." }
            div { class: "flex gap-2",
                button {
                    class: "btn btn-ghost",
                    disabled: saving,
                    onclick: move |_| on_discard.call(()),
                    "discard"
                }
                button {
                    class: "btn btn-primary",
                    disabled: saving,
                    onclick: move |_| on_save.call(()),
                    if saving {
                        Spinner {}
                    }
                    "save"
                }
            }
        }
    }
}
