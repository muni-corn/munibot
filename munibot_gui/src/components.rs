use dioxus::prelude::*;
use munibot_api::server_fns::auth::get_authenticated_user;

use crate::app::Route;

/// Shows the current sign-in state: a "sign in with discord" link when
/// signed out, or a greeting, a link to the dashboard, and a sign-out link
/// when signed in.
///
/// `/auth/discord/authorize` and `/auth/logout` are plain server routes, not
/// dioxus router routes, so those are ordinary `a` tags rather than `Link`s
/// -- the browser needs to actually navigate (and follow discord's redirect
/// chain), not perform a client-side route change. `/dashboard` is a real
/// dioxus route, so that one is a `Link`.
#[component]
pub fn AccountStatus() -> Element {
    let user = use_resource(get_authenticated_user);

    match &*user.read() {
        Some(Ok(Some(user))) => {
            let name = user.display_name.clone();
            rsx! {
                span { "hi, {name}! ^-^ " }
                Link { to: Route::Dashboard {}, "your servers" }
                " · "
                a { href: "/auth/logout", "sign out" }
            }
        }
        _ => rsx! {
            a { class: "btn btn-primary", href: "/auth/discord/authorize", "Sign in with discord" }
        },
    }
}

#[component]
pub fn Spinner() -> Element {
    rsx! {
        i {
            font_size: "24pt",
            color: "white",
            class: "animate-spin me-2 ph-duotone ph-circle-notch",
        }
    }
}
