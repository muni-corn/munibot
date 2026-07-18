use dioxus::prelude::*;

use crate::app::Route;

static LOGO: Asset = asset!("assets/munibot_icon.png");

#[component]
pub fn HomeLayout() -> Element {
    rsx! {
        div{
            class: "flex flex-col w-screen h-screen",
            div {
                class: "flex flex-row items-center gap-4 p-4 place-self-start",
                img {
                    src: LOGO,
                    class: "rounded-full w-16"
                }
                span {
                    class: "font-black font-mono text-3xl",
                    "munibot"
                }
            }
            div {
                class: "grow flex flex-row",
                div {
                    class: "w-24"
                }
                div {
                    class: "grow bg-slate-950/50 sm:rounded-ss-3xl",
                    Outlet::<Route> {}
                }
            }
        }
    }
}
