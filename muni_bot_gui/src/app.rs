use crate:: error_template::{AppError, ErrorTemplate};
use leptos::*;
use leptos_meta::*;
use leptos_router::*;

use crate::get::Get;

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/muni_bot_gui.css"/>

        // sets the document title
        <Title text="muni_bot"/>

        // content for this welcome page
        <Router fallback=|| {
            let mut outside_errors = Errors::default();
            outside_errors.insert_with_default_key(AppError::NotFound);
            view! { <ErrorTemplate outside_errors/> }.into_view()
        }>
            <main>
                <Routes>
                    <Route path="" view=HomePage/>
                    <Route path="/get" view=Get/>
                </Routes>
            </main>
        </Router>
    }
}

/// Renders the home page of your application.
#[component]
fn HomePage() -> impl IntoView {
    let discord_oauth_url = std::env::var("DISCORD_OAUTH_URL").expect("DISCORD_OAUTH_URL env var not found :c");

    view! {
        <div class="h-screen w-screen bg-black">
            <div class="flex items-center flex-col h-screen w-screen bg-muni-bot bg-cover bg-center bg-blend-multiply bg-black/75 place-content-center">
                <h1 class="text-center mb-8 text-sky-400">"muni_bot"</h1>
                <p class="text-center text-sky-400 mb-20">
                    "the fun bot for twitch and discord. personality included."
                </p>
                <A href=discord_oauth_url>
                    <button class="btn raised mx-auto mb-4">"Get muni_bot"</button>
                </A>
                <A href="dashboard">
                    <button class="btn mx-auto">"Dashboard"</button>
                </A>
            </div>

        </div>
    }
}
