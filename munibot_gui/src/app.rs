mod route;

use dioxus::prelude::*;
pub use route::Route;

/// Application root — mounts the router and injects global head elements.
#[component]
pub fn App() -> Element {
    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        document::Stylesheet { href: "https://cdn.jsdelivr.net/npm/@phosphor-icons/web@2.1.2/src/duotone/style.css" }
        Router::<Route> {}
    }
}

/// Top-level layout wrapping every route.
#[component]
fn MainLayout() -> Element {
    rsx! {
        main { class: "min-h-screen bg-radial-[circle_at_100%_100%] from-sky-500 via-emerald-700 to-slate-900 text-white",
            Outlet::<Route> {}
        }
    }
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        document::Title { "not found ~ munibot" }
        div { class: "flex h-screen flex-col place-content-center items-center gap-4",
            h1 { class: "text-6xl font-bold", "404" }
            p { class: "text-slate-300", "there's nothing here for path /{segments.join(\"/\")}" }
        }
    }
}
