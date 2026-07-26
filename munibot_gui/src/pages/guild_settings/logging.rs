use dioxus::prelude::*;

/// A guild's logging settings: which channel (if any) server events are
/// logged to.
#[component]
pub fn LoggingSettingsPage(guild_id: String) -> Element {
    rsx! {
        document::Title { "logging settings ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "font-black text-2xl", "logging" }
            p { class: "text-slate-300", "logging settings for server {guild_id}." }
        }
    }
}
