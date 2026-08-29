use dioxus::prelude::*;

use crate::app::Route;

pub mod ai;
pub mod logging;

/// Lists the settings sections available for a guild.
#[component]
pub fn GuildSettings(guild_id: String) -> Element {
    rsx! {
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "server settings" }
            ul { class: "menu w-full max-w-xs gap-1 rounded-box bg-slate-900/50 p-2",
                li {
                    Link {
                        to: Route::GuildLoggingSettings {
                            guild_id: guild_id.clone(),
                        },
                        "logging"
                    }
                }
                li {
                    Link {
                        to: Route::GuildAiSettings {
                            guild_id: guild_id.clone(),
                        },
                        "ai"
                    }
                }
            }
        }
    }
}
