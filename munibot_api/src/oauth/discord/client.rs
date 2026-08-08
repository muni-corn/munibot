//! A single, shared `reqwest::Client` for every discord api call munibot
//! makes -- oauth token exchanges, oauth rest calls, and bot-token rest
//! calls alike.
//!
//! Each `reqwest::Client` owns its own connection pool; building a fresh one
//! per request (as `oauth/discord.rs` and `oauth/discord/bot.rs` used to)
//! throws away keep-alive to `discord.com` on every single call. This also
//! gives discord a proper `User-Agent`, which they ask api consumers to
//! send.
use std::{sync::LazyLock, time::Duration};

use reqwest::Client;

/// A reasonable ceiling for a single discord api round trip. Retries (see
/// `rate_limit`) apply on top of this per-attempt timeout, not around it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .user_agent(concat!(
            "munibot/",
            env!("CARGO_PKG_VERSION"),
            " (+https://codeberg.org/municorn/munibot)"
        ))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("static client configuration is valid")
});

/// The shared client used for all discord api requests.
pub fn client() -> &'static Client {
    &CLIENT
}
