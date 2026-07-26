use dioxus::prelude::*;

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
