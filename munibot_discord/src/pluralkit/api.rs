use std::time::Duration;

use reqwest::{Client, StatusCode};
use tracing::warn;

use super::models::PkMessage;

const PK_API_BASE: &str = "https://api.pluralkit.me/v2";

/// User-Agent sent with every PluralKit API request.
///
/// PluralKit asks API consumers to include a descriptive User-Agent so they can
/// identify traffic and reach out if a client misbehaves.
const USER_AGENT: &str = concat!(
    "munibot/",
    env!("CARGO_PKG_VERSION"),
    " (https://git.musicaloft.com/municorn/munibot)",
);

/// How long to wait before retrying a 404 response.
///
/// When PluralKit deletes the original message immediately after creating the
/// proxy, there can be a small race between the delete event arriving and PK
/// indexing the message. A single short retry handles this without adding
/// noticeable latency for genuine (non-PK) deletions.
const RETRY_DELAY: Duration = Duration::from_millis(1000);

/// The outcome of a PluralKit message lookup.
#[derive(Debug)]
pub enum PkLookup {
    /// The message ID is known to PluralKit and the metadata was retrieved.
    Proxied(PkMessage),

    /// PluralKit has no record of this message; it is not a proxy trigger or
    /// proxy webhook message.
    NotProxied,

    /// The lookup could not be completed due to a network error, rate limit, or
    /// unexpected response. The caller should fail open and log normally.
    Unavailable,
}

/// A lightweight client for the PluralKit v2 API.
///
/// Wraps a shared `reqwest::Client` and provides a single method for looking up
/// whether a Discord message was part of a PluralKit proxy interaction.
#[derive(Clone, Debug)]
pub struct PkClient {
    client: Client,
}

impl Default for PkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PkClient {
    /// Creates a new `PkClient` with the shared HTTP client and the correct
    /// `User-Agent` header.
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            // reqwest only fails to build a Client when TLS initialization fails,
            // which would be a fatal startup error anyway
            .expect("failed to build reqwest client for pluralkit");

        Self { client }
    }

    /// Looks up a message by ID in the PluralKit API.
    ///
    /// When `retry` is true, a single 404 response is retried after a short
    /// delay. Use this for delete-trigger lookups where a race between the
    /// delete event and PK indexing the message is possible. Set it to false
    /// when looking up by the proxy message ID directly, since those are
    /// guaranteed to be indexed already.
    pub async fn lookup_message(&self, message_id: &str, retry: bool) -> PkLookup {
        let result = self.fetch_message(message_id).await;

        match result {
            Ok(Some(msg)) => PkLookup::Proxied(msg),
            Ok(None) if retry => {
                // wait briefly and try once more in case PK hasn't indexed yet
                tokio::time::sleep(RETRY_DELAY).await;
                match self.fetch_message(message_id).await {
                    Ok(Some(msg)) => PkLookup::Proxied(msg),
                    Ok(None) => PkLookup::NotProxied,
                    Err(e) => {
                        warn!("pluralkit api retry failed for message {message_id}: {e}");
                        PkLookup::Unavailable
                    }
                }
            }
            Ok(None) => PkLookup::NotProxied,
            Err(e) => {
                warn!("pluralkit api lookup failed for message {message_id}: {e}");
                PkLookup::Unavailable
            }
        }
    }

    /// Performs the raw GET request. Returns `Ok(Some(_))` on 200, `Ok(None)`
    /// on 404, and `Err` on anything else.
    async fn fetch_message(&self, message_id: &str) -> Result<Option<PkMessage>, FetchError> {
        let url = format!("{PK_API_BASE}/messages/{message_id}");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(FetchError::Request)?;

        match response.status() {
            StatusCode::OK => {
                let msg = response
                    .json::<PkMessage>()
                    .await
                    .map_err(FetchError::Request)?;
                Ok(Some(msg))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(FetchError::Unexpected(status)),
        }
    }
}

/// Internal error type for `fetch_message`.
#[derive(Debug)]
enum FetchError {
    Request(reqwest::Error),
    Unexpected(StatusCode),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Request(e) => write!(f, "request error: {e}"),
            FetchError::Unexpected(s) => write!(f, "unexpected status {s}"),
        }
    }
}
