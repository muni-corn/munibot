use dioxus::prelude::*;
use munibot_api::{guilds::GuildSummary, server_fns::guilds::get_guilds};

use crate::{app::Route, components::Spinner};

/// Layout for every `/dashboard/*` route: shows the sidebar of discord
/// servers the signed-in user owns or administrates, with the current
/// route's content (the dashboard index, or a guild's settings pages) in
/// the outlet beside it.
#[component]
pub fn Dashboard() -> Element {
    let guilds = use_resource(get_guilds);

    let content = match &*guilds.read() {
        Some(Ok(guilds)) if guilds.is_empty() => rsx! {
            p { class: "p-4 text-slate-300",
                "you don't own or manage any discord servers munibot can see yet."
            }
        },
        Some(Ok(guilds)) => rsx! {
            div { class: "flex h-full flex-row",
                DiscordSidebar { guilds: guilds.to_vec() }
                div { class: "grow bg-slate-950/50 sm:rounded-ss-3xl", Outlet::<Route> {} }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "flex h-full grow flex-col place-content-center items-center gap-4",
                h2 { class: "text-3xl font-black", "who are you?" }
                p { "you need to sign in to see your servers." }
                a { href: "/auth/discord/authorize", class: "btn btn-primary",
                    "Sign in with discord"
                }
            }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "your servers ~ munibot" }
        {content}
    }
}

/// Content shown in `Dashboard`'s outlet at the bare `/dashboard` path,
/// before a server has been picked from the sidebar.
#[component]
pub fn DashboardIndex() -> Element {
    rsx! {
        div { class: "flex h-full flex-col place-content-center items-center gap-2 p-4 text-slate-300",
            p { "pick a server from the sidebar to manage its settings." }
        }
    }
}

#[component]
pub fn DiscordSidebar(guilds: Vec<GuildSummary>) -> Element {
    rsx! {
        div { class: "flex h-full w-20 flex-col items-center gap-4 p-4 sm:rounded-ss-3xl",

            ul { class: "flex flex-col gap-4",
                for guild in guilds.iter() {
                    GuildRow { key: "{guild.id}", guild: guild.clone() }
                }
            }
        }
    }
}

#[component]
fn GuildRow(guild: GuildSummary) -> Element {
    let icon = if let Some(icon_url) = &guild.icon_url {
        rsx! {
            img { src: icon_url.clone(), class: "size-12 rounded-full" }
        }
    } else {
        rsx! {
            div { class: "flex size-12 items-center justify-center rounded-full bg-slate-800 font-bold text-slate-300",
                "{guild.name.chars().next().unwrap_or('?')}"
            }
        }
    };

    rsx! {
        div { class: "tooltip tooltip-right", "data-tip": guild.name,
            Link {
                to: Route::GuildSettings {
                    guild_id: guild.id.clone(),
                },
                {icon}
            }
        }
    }
}
