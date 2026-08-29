// munibot_api: the rpc boundary between munibot's gui and its data.
//
// dual-target, like munibot itself: the `web` feature builds the wasm
// client stubs and wire dtos, and the `server` feature builds the actual
// server function bodies plus everything native-only (db access, the
// discord oauth client, axum session/auth glue) that can't compile for
// wasm32.

pub mod auth;
pub mod chat;
pub mod guilds;
pub mod pipeline;

#[cfg(feature = "server")]
pub mod mailer;
#[cfg(feature = "server")]
pub mod oauth;
pub mod server_fns;
pub mod settings;
