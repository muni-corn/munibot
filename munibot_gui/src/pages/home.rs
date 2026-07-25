use dioxus::prelude::*;

use crate::components::AccountStatus;

/// Landing page.
#[component]
pub fn Home() -> Element {
    rsx! {
        document::Title { "munibot" }
        div { class: "flex h-screen flex-col place-content-center items-center gap-4",
            h1 { class: "text-5xl font-bold", "hi, i'm munibot! ^-^" }
            p { class: "text-slate-300", "the universe's most lovable bot, personality included." }
            AccountStatus {}
        }
    }
}
