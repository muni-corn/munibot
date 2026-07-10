use dioxus::prelude::*;

use crate::api::server_fns::guilds::get_guilds;

/// Shows the discord servers the signed-in user owns or administrates.
#[component]
pub fn Dashboard() -> Element {
    let guilds = use_resource(get_guilds);

    rsx! {
        document::Title { "your servers ~ munibot" }
        div { class: "flex flex-col gap-6 items-center py-16 min-h-screen",
            h1 { class: "text-4xl font-bold", "your servers" }
            {match &*guilds.read() {
                Some(Ok(guilds)) if guilds.is_empty() => rsx! {
                    p { class: "text-slate-300",
                        "you don't own or manage any discord servers munibot can see yet."
                    }
                },
                Some(Ok(guilds)) => rsx! {
                    ul { class: "flex flex-col gap-3 w-full max-w-md",
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
                    p { class: "text-slate-300", "loading..." }
                },
            }}
        }
    }
}

#[component]
fn GuildRow(guild: crate::api::guilds::GuildSummary) -> Element {
    rsx! {
        li { class: "flex gap-3 items-center p-3 bg-slate-800 rounded-lg",
            if let Some(icon_url) = &guild.icon_url {
                img { src: icon_url.clone(), class: "w-10 h-10 rounded-full" }
            } else {
                div { class: "flex justify-center items-center w-10 h-10 font-bold bg-slate-700 rounded-full text-slate-300",
                    "{guild.name.chars().next().unwrap_or('?')}"
                }
            }
            span { class: "font-bold", "{guild.name}" }
        }
    }
}
