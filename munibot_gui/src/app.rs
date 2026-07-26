use dioxus::prelude::*;

use crate::{
    layouts::home::HomeLayout,
    pages::{
        dashboard::{Dashboard, DashboardIndex},
        guild_settings::{GuildSettings, logging::LoggingSettingsPage},
        home::Home,
    },
};

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
        #[layout(HomeLayout)]
            #[layout(Dashboard)]
                #[route("/dashboard")]
                DashboardIndex {},
                #[route("/dashboard/:guild_id")]
                GuildSettings { guild_id: String },
                // renders LoggingSettingsPage rather than a component named
                // GuildLoggingSettings, so this variant's name doesn't
                // collide with munibot_api::settings::GuildLoggingSettings
                #[route("/dashboard/:guild_id/logging", LoggingSettingsPage)]
                GuildLoggingSettings { guild_id: String },
            #[end_layout]
        #[end_layout]
    #[end_layout]

    // catch-all — must be outside the layout so 404 is not wrapped
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
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
