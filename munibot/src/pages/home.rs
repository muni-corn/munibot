use dioxus::prelude::*;

/// Landing page.
#[component]
pub fn Home() -> Element {
    rsx! {
        document::Title { "munibot" }
        div { class: "flex flex-col place-content-center items-center gap-4 h-screen",
            h1 { class: "text-5xl font-bold", "hi, i'm munibot! ^-^" }
            p { class: "text-slate-300", "the universe's most lovable bot, personality included." }
        }
    }
}
