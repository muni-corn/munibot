use dioxus::prelude::*;

use crate::api::server_fns::auth::get_authenticated_user;

/// Shows the current sign-in state: a "sign in with discord" link when
/// signed out, or a greeting and a sign-out link when signed in.
///
/// `/auth/discord/authorize` and `/auth/logout` are plain server routes, not
/// dioxus router routes, so these are ordinary `a` tags rather than `Link`s
/// -- the browser needs to actually navigate (and follow discord's redirect
/// chain), not perform a client-side route change.
#[component]
pub fn AccountStatus() -> Element {
    let user = use_resource(get_authenticated_user);

    match &*user.read() {
        Some(Ok(Some(user))) => {
            let name = user.display_name.clone();
            rsx! {
                span { "hi, {name}! ^-^ " }
                a { href: "/auth/logout", "sign out" }
            }
        }
        _ => rsx! {
            a { href: "/auth/discord/authorize", "sign in with discord" }
        },
    }
}
