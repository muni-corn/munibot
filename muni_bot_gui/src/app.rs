use leptos::*;
use leptos_meta::*;
use leptos_router::*;

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/muni_bot_gui.css"/>

        // sets the document title
        <Title text="muni_bot"/>

        // content for this welcome page
        <Router>
            <main>
                <Routes>
                    <Route path="" view=HomePage/>
                    <Route path="/*any" view=NotFound/>
                </Routes>
            </main>
        </Router>
    }
}

/// Renders the home page of your application.
#[component]
fn HomePage() -> impl IntoView {
    view! {
        <div class="h-screen w-screen bg-black">
            <div class="flex items-center flex-col h-screen w-screen bg-muni-bot bg-cover bg-center bg-blend-multiply bg-black/75 place-content-center">
                <p class="text-center text-5xl font-bold font-mono mb-8 text-sky-400">"muni_bot"</p>
                <p class="text-center text-sky-400 mb-20">
                    "the best bot for twitch and discord. personality included."
                </p>
                <button class="text-white font-bold align-center py-3 px-6 rounded-full bg-emerald-500 place-self-center mx-auto">
                    "Get muni_bot"
                </button>
            </div>

        </div>
    }
}

/// 404 - Not Found
#[component]
fn NotFound() -> impl IntoView {
    // set an HTTP status code 404
    // this is feature gated because it can only be done during
    // initial server-side rendering
    // if you navigate to the 404 page subsequently, the status
    // code will not be set because there is not a new HTTP request
    // to the server
    #[cfg(feature = "ssr")]
    {
        // this can be done inline because it's synchronous
        // if it were async, we'd use a server function
        let resp = expect_context::<leptos_actix::ResponseOptions>();
        resp.set_status(actix_web::http::StatusCode::NOT_FOUND);
    }

    view! { <h1>"Not Found"</h1> }
}
