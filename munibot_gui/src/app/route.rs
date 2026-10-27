// the `Routable` derive wraps every dynamic segment's parse error as
// `XParseError::fieldParseError(err)`; for `String` and `Vec<String>` segments
// that error is `Infallible`, and recent rustc flags any call taking an
// uninhabited argument as unreachable. lint attributes on the enum itself
// don't reach the derive's generated impls, so the allow has to be at
// module level, which is why this enum gets a module to itself
#![allow(unreachable_code)]

use dioxus::prelude::*;

use super::{MainLayout, NotFound};
use crate::{
    layouts::home::HomeLayout,
    pages::{
        dashboard::{Dashboard, DashboardIndex},
        guild_settings::{GuildSettings, logging::LoggingSettingsPage},
        home::Home,
    },
};

/// Application route enum.
///
/// Every page reachable via the browser URL is declared here. The enum is
/// derived with `Routable` so Dioxus can generate typesafe `Link` targets and
/// match incoming paths automatically.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    // catch-all -- declared before any layout so 404 is not wrapped. the
    // derive sorts routes static-first and catch-all-last when matching, so
    // declaring it first doesn't shadow anything below
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },

    // layouts below stay open through the end of the enum, so every variant
    // added after this point renders inside MainLayout
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
                GuildLoggingSettings { guild_id: String }
}
