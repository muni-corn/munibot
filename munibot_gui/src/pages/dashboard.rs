use dioxus::prelude::*;
use munibot_api::server_fns::guilds::get_guilds;

use crate::{app::Route, components::Spinner};

/// Shows the discord servers the signed-in user owns or administrates.
#[component]
pub fn Dashboard() -> Element {
    rsx! {
        document::Title { "your servers ~ munibot" }
        div {
            class: "flex flex-row h-full",
            DiscordSidebar {}
            div {
                class: "grow bg-slate-950/50 sm:rounded-ss-3xl",
                Outlet::<Route> {}
            }
        }
    }
}

#[component]
pub fn DiscordSidebar() -> Element {
    let guilds = use_resource(get_guilds);

    rsx! {
        div { class: "w-20 p-4 sm:rounded-ss-3xl flex flex-col gap-4 items-center h-full",
            {match &*guilds.read() {
                Some(Ok(guilds)) if guilds.is_empty() => rsx! {
                    p { class: "text-slate-300 p-4",
                        "you don't own or manage any discord servers munibot can see yet."
                    }
                },
                Some(Ok(guilds)) => rsx! {
                    ul { class: "flex flex-col gap-4",
                        for guild in guilds.iter() {
                            GuildRow { key: "{guild.id}", guild: guild.clone() }
                        }
                    }
                },
                Some(Err(_)) => rsx! {
                    div { class: "flex flex-col gap-2 items-center",
                        p { class: "text-slate-300", "you need to sign in to see your servers." }
                        a { href: "/auth/discord/authorize", class: "underline", "sign in with discord" }
                    }
                },
                None => rsx! {
                    Spinner {}
                },
            }}
        }
    }
}

#[component]
fn GuildRow(guild: munibot_api::guilds::GuildSummary) -> Element {
    rsx! {
        if let Some(icon_url) = &guild.icon_url {
            img { src: icon_url.clone(), class: "size-12 rounded-full" }
        } else {
            div { class: "flex justify-center items-center size-12 font-bold bg-slate-800 rounded-full text-slate-300",
                "{guild.name.chars().next().unwrap_or('?')}"
            }
        }
    }
}
