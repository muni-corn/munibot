use leptos::*;
use leptos_router::*;
use leptos_use::{use_cookie, utils::FromToStringCodec};

#[derive(Default, Params, PartialEq)]
struct DiscordAuthQuery {
    code: String,
}

#[component]
pub fn DiscordAuth() -> impl IntoView {
    let query = use_query::<DiscordAuthQuery>();

    let (token, set_token) = use_cookie::<String, FromToStringCodec>("token");

    let code = move || query.with(|q| q.as_ref().map(|q| q.code.clone()).unwrap_or_default());

    view! {
        <div class="backdrop-blue-green">
            <h1>Discord Auth</h1>
            <h2>Code: {code()}</h2>
        </div>
    }
}
