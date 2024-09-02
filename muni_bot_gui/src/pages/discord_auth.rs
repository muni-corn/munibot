use leptos::*;
use leptos_router::*;

use crate::auth::login_discord;

#[derive(Default, Params, PartialEq)]
struct DiscordAuthQuery {
    code: String,
}

#[component]
pub fn DiscordAuth() -> impl IntoView {
    let query = use_query::<DiscordAuthQuery>();
    let code = move || query.with(|q| q.as_ref().map(|q| q.code.clone()).unwrap_or_default());

    // set token
    let f = move || async move { login_discord(code()).await.err() };

    view! {
        <Await future=f let:result>
            {match result {
                None => {
                    view! {
                        <div>
                            <Redirect path="/dashboard"/>
                        </div>
                    }
                }
                Some(e) => view! { <div class="error-backdrop-container">{e.to_string()}</div> },
            }}

        </Await>
    }
}
