use dioxus::prelude::*;
use munibot_api::settings::ChannelSummary;

use crate::components::Spinner;

/// A titled group of related settings within a settings page, e.g.
/// "logging" or "autodelete".
#[component]
pub fn SettingsSection(title: String, description: Option<String>, children: Element) -> Element {
    rsx! {
        section { class: "flex flex-col gap-4 rounded-box bg-slate-900/50 p-6",
            div { class: "flex flex-col gap-1",
                h3 { class: "text-xl font-black", {title} }
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
        div { class: "flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between",
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
        div { class: "sticky bottom-0 flex items-center justify-between gap-4 rounded-box bg-slate-800 p-4 shadow-lg",
            span { class: "text-sm text-slate-300", "you have unsaved changes." }
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

/// A `<select>` for choosing one of a guild's text channels, or none,
/// grouped by category. `value` and `on_change` carry channel ids as
/// strings, matching `ChannelSummary::id` -- an empty selection means no
/// channel is chosen.
#[component]
pub fn ChannelSelect(
    channels: Vec<ChannelSummary>,
    value: Option<String>,
    none_label: String,
    on_change: EventHandler<Option<String>>,
) -> Element {
    // group consecutive channels sharing a category -- the server already
    // sorts these so channels in the same category are contiguous
    let mut groups: Vec<(Option<String>, Vec<ChannelSummary>)> = Vec::new();
    for channel in channels {
        match groups.last_mut() {
            Some((category, items)) if *category == channel.category => items.push(channel),
            _ => groups.push((channel.category.clone(), vec![channel])),
        }
    }

    rsx! {
        select {
            class: "select w-full",
            value: value.unwrap_or_default(),
            onchange: move |event| {
                let selected = event.value();
                on_change.call(if selected.is_empty() { None } else { Some(selected) });
            },
            option { value: "", {none_label} }
            for (category, items) in groups {
                if let Some(category) = category {
                    optgroup { label: category,
                        for channel in items {
                            option { value: "{channel.id}", "#{channel.name}" }
                        }
                    }
                } else {
                    for channel in items {
                        option { value: "{channel.id}", "#{channel.name}" }
                    }
                }
            }
        }
    }
}

/// Shown in place of a guild's settings when munibot hasn't been invited to
/// that server yet -- a real, distinct state, not a generic error.
#[component]
pub fn InviteMunibotPrompt(invite_link: Option<String>) -> Element {
    rsx! {
        div { class: "flex flex-col items-center gap-4 p-6 text-center",
            h3 { class: "text-xl font-black", "munibot isn't here yet" }
            p { class: "text-slate-400", "invite munibot to this server to manage its settings." }
            if let Some(invite_link) = invite_link {
                a {
                    class: "btn btn-primary",
                    href: invite_link,
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "invite munibot"
                }
            }
        }
    }
}
