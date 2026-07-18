// munibot_gui: munibot's gui, with a face to match.
//
// dual-target, like munibot_api: the `web` feature builds the wasm client
// (hydrated into the page by dioxus), and the `server` feature builds the
// axum server that renders the gui, serves munibot_api's server functions,
// and backs login sessions -- everything native-only is gated behind it so
// the wasm build never sees it.

pub mod app;
pub mod components;
pub mod layouts;
pub mod pages;

#[cfg(feature = "server")]
pub mod server;

/// Launches the wasm client, hydrating into the page rendered by the
/// server half of this app.
///
/// Only compiled without the `server` feature, since it's the web entry
/// point -- the server half serves the gui through [`app::App`] directly
/// (see the `server` module).
#[cfg(not(feature = "server"))]
pub fn launch_web() {
    dioxus::launch(app::App);
}
