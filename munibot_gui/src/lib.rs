// munibot_gui: munibot's gui, with a face to match.
//
// dual-target, like munibot_api: the `web` feature builds the wasm client
// (hydrated into the page by dioxus), and the `server` feature builds the
// axum server that renders the gui, serves munibot_api's server functions,
// and backs login sessions -- everything native-only is gated behind it so
// the wasm build never sees it.

pub mod app;
pub mod components;
pub mod pages;
