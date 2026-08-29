use dioxus::prelude::*;
use munibot_api::{
    auth::LinkedAccountSummary,
    server_fns::auth::{list_linked_accounts, unlink_account},
};

use crate::components::{SignInLinks, Spinner};

/// Every provider linked to the signed-in user's account, with a way to
/// link another and to unlink one already there.
///
/// The unlink button is disabled once only one account remains -
/// `unlink_account` itself refuses that too, but disabling it here means
/// a person sees *why* immediately, rather than clicking through to a
/// server error - the same "surface the constraint, don't just enforce
/// it" reasoning the settings pages elsewhere already follow.
#[component]
pub fn Account() -> Element {
    let mut accounts = use_resource(list_linked_accounts);
    let mut error = use_signal(|| None::<String>);

    let content = match &*accounts.read() {
        Some(Ok(linked)) => {
            let only_one_left = linked.len() <= 1;
            rsx! {
                ul { class: "flex flex-col gap-2",
                    for account in linked.iter() {
                        LinkedAccountRow {
                            key: "{account.provider}",
                            account: account.clone(),
                            disabled: only_one_left,
                            on_unlinked: move |_| accounts.restart(),
                            on_error: move |message| error.set(Some(message)),
                        }
                    }
                }
                div { class: "mt-2",
                    h3 { class: "mb-2 font-semibold", "link another way to sign in" }
                    SignInLinks {}
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load your linked accounts :< {e}" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "account ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "your account" }
            p { class: "text-sm text-slate-400",
                "every provider linked below signs in to the exact same munibot account - the \
                 same memories, conversations, and settings, regardless of which one you use."
            }
            if let Some(message) = &*error.read() {
                div { class: "alert alert-error", {message.clone()} }
            }
            {content}
        }
    }
}

#[component]
fn LinkedAccountRow(
    account: LinkedAccountSummary,
    disabled: bool,
    on_unlinked: EventHandler<()>,
    on_error: EventHandler<String>,
) -> Element {
    let provider = account.provider.clone();
    let unlink = move |_| {
        let provider = provider.clone();
        spawn(async move {
            match unlink_account(provider).await {
                Ok(()) => on_unlinked.call(()),
                Err(e) => on_error.call(e.to_string()),
            }
        });
    };

    rsx! {
        li { class: "flex items-center justify-between gap-4 rounded-box bg-slate-900/50 p-3",
            div { class: "flex flex-col",
                span { class: "font-semibold capitalize", {account.provider.clone()} }
                span { class: "text-sm text-slate-400", {account.username.clone()} }
            }
            button {
                class: "btn btn-ghost btn-sm",
                disabled,
                title: if disabled { "you can't unlink your last remaining sign-in method" } else { "" },
                onclick: unlink,
                "unlink"
            }
        }
    }
}
