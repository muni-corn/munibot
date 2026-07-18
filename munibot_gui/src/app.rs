use dioxus::prelude::*;

use crate::pages::{dashboard::Dashboard, home::Home};

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
        main { class: "min-h-screen bg-slate-900 text-white", Outlet::<Route> {} }
    }
}

/// Application route enum.
///
/// Every page reachable via the browser URL is declared here. The enum is
/// derived with `Routable` so Dioxus can generate typesafe `Link` targets and
/// match incoming paths automatically.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(MainLayout)]
        #[route("/")]
        Home,
        #[route("/dashboard")]
        Dashboard,
    #[end_layout]

    // catch-all — must be outside the layout so 404 is not wrapped
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        document::Title { "not found ~ munibot" }
        div { class: "flex flex-col place-content-center items-center gap-4 h-screen",
            h1 { class: "text-6xl font-bold", "404" }
            p { class: "text-slate-300", "there's nothing here for path /{segments.join(\"/\")}" }
        }
    }
}
