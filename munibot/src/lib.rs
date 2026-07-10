// munibot: the universe's most lovable bot, with a face to match.
//
// this crate compiles to two very different targets depending on which cargo
// feature is active:
//   - `web`: the wasm client, hydrated into the page by dioxus
//   - `server`: the axum server that renders the gui, serves server functions,
//     and runs the discord/twitch bots alongside it

#[cfg(feature = "server")]
pub mod bot;

pub mod app;
pub mod pages;
