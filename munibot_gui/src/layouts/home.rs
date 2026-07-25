use dioxus::prelude::*;

use crate::app::Route;

static LOGO: Asset = asset!("assets/munibot_icon.png");

#[component]
pub fn HomeLayout() -> Element {
    rsx! {
        div { class: "flex h-screen w-screen flex-col",
            div { class: "flex flex-row items-center gap-4 place-self-start p-4",
                img { src: LOGO, class: "w-16 rounded-full" }
                span { class: "font-mono text-3xl font-black", "munibot" }
            }
            div { class: "flex grow flex-row",
                div { class: "w-24" }
                div { class: "grow bg-slate-950/50 sm:rounded-ss-3xl", Outlet::<Route> {} }
            }
        }
    }
}
