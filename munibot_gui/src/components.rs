use dioxus::prelude::*;
use munibot_api::server_fns::auth::get_authenticated_user;

use crate::app::Route;

pub mod chat;
pub mod settings;

/// Shows the current sign-in state: "sign in with" links for every
/// configured provider when signed out, or a greeting, a link to the
/// dashboard, and a sign-out link when signed in.
///
/// `/auth/<provider>/authorize` and `/auth/logout` are plain server routes,
/// not dioxus router routes, so those are ordinary `a` tags rather than
/// `Link`s -- the browser needs to actually navigate (and follow a
/// provider's redirect chain), not perform a client-side route change.
/// `/dashboard` is a real dioxus route, so that one is a `Link`.
#[component]
pub fn AccountStatus() -> Element {
    let user = use_resource(get_authenticated_user);

    match &*user.read() {
        Some(Ok(Some(user))) => {
            let name = user.display_name.clone();
            rsx! {
                span { "hi, {name}! ^-^ " }
                Link { to: Route::Chat {}, "chat" }
                " · "
                Link { to: Route::Personas {}, "personas" }
                " · "
                Link { to: Route::DashboardIndex {}, "your servers" }
                " · "
                Link { to: Route::Memory {}, "memory" }
                " · "
                Link { to: Route::Usage {}, "usage" }
                " · "
                Link { to: Route::Pipelines {}, "pipelines" }
                " · "
                a { href: "/auth/logout", "sign out" }
            }
        }
        _ => rsx! {
            SignInLinks {}
        },
    }
}

/// Every "sign in with <provider>" link, shown wherever signing in is
/// offered -- the header (via `AccountStatus`) and the dashboard's own
/// signed-out state both use this rather than duplicating the list.
#[component]
pub fn SignInLinks() -> Element {
    rsx! {
        div { class: "flex flex-wrap gap-2",
            a { class: "btn btn-primary", href: "/auth/discord/authorize", "Sign in with discord" }
            a { class: "btn btn-primary", href: "/auth/github/authorize", "Sign in with github" }
        }
    }
}

#[component]
pub fn Spinner() -> Element {
    rsx! {
        i {
            font_size: "24pt",
            color: "white",
            class: "me-2 animate-spin ph-duotone ph-circle-notch",
        }
    }
}
