use dioxus::prelude::*;
use munibot_api::{guilds::GuildSummary, server_fns::guilds::get_guilds};

use crate::{app::Route, components::Spinner};

/// Shows the discord servers the signed-in user owns or administrates.
#[component]
pub fn Dashboard() -> Element {
    let guilds = use_resource(get_guilds);

    let content = match &*guilds.read() {
        Some(Ok(guilds)) if guilds.is_empty() => rsx! {
            p { class: "text-slate-300 p-4",
                "you don't own or manage any discord servers munibot can see yet."
            }
        },
        Some(Ok(guilds)) => rsx! {
            div {
                class: "flex flex-row h-full",
                DiscordSidebar { guilds: guilds.to_vec() }
                div {
                    class: "grow bg-slate-950/50 sm:rounded-ss-3xl",
                    Outlet::<Route> {}
                }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "grow flex flex-col gap-4 place-content-center items-center h-full",
                h2 { class: "font-black text-3xl", "who are you?" }
                p { "you need to sign in to see your servers." }
                a { href: "/auth/discord/authorize", class: "btn btn-primary", "Sign in with discord" }
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

#[component]
pub fn DiscordSidebar(guilds: Vec<GuildSummary>) -> Element {
    rsx! {
        div { class: "w-20 p-4 sm:rounded-ss-3xl flex flex-col gap-4 items-center h-full",

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
            div {
                class: "flex justify-center items-center size-12 font-bold bg-slate-800 rounded-full text-slate-300",
                "{guild.name.chars().next().unwrap_or('?')}"
            }
        }
    };

    rsx! {
        div {
            class: "tooltip tooltip-right cursor-pointer",
            "data-tip": guild.name,
            {icon}
        }
    }
}
