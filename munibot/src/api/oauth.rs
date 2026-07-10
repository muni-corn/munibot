// oauth client code is entirely server-side: it exchanges secrets and
// bearer tokens that must never reach the wasm client.

pub mod discord;
pub mod routes;
