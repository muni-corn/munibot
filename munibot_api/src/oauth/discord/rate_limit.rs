//! Parsing discord's `429` responses, and retrying requests that hit one.
//!
//! Discord expects api consumers to back off using `Retry-After` rather
//! than hammering a route until it succeeds (see discord's rate limits
//! docs); this module is the one place that logic lives, shared by both the
//! oauth client (`oauth/discord.rs`) and the bot-token client
//! (`oauth/discord/bot.rs`).
use std::time::Duration;

use reqwest::{RequestBuilder, Response, StatusCode, header::RETRY_AFTER};
use serde::Deserialize;

/// How many times to retry a request that comes back `429`, on top of the
/// first attempt.
const MAX_RETRIES: u32 = 3;

/// A ceiling on how long a single retry will sleep for. If discord asks for
/// longer than this (its rate limit responses have been observed asking for
/// upwards of ten minutes -- see discord-api-docs#670), it isn't reasonable
/// to hold a server function open waiting it out; the caller gets a
/// `SendError::RateLimited` immediately instead.
const MAX_SLEEP: Duration = Duration::from_secs(5);

/// A parsed discord rate limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    /// How long discord is asking callers to wait before retrying.
    pub retry_after: Duration,
    /// Whether this is a shared, ip-wide limit rather than a per-route or
    /// per-user one.
    pub global: bool,
}

/// The json body discord sends alongside a `429`, duplicating the
/// `Retry-After` header and adding the `global` flag.
#[derive(Debug, Deserialize)]
struct RateLimitBody {
    retry_after: f64,
    #[serde(default)]
    global: bool,
}

/// Parses a rate limit out of a response's already-extracted `Retry-After`
/// header value and body text. Split out from `from_response` so it can be
/// unit tested without building a real `reqwest::Response`.
fn parse_parts(retry_after_header: Option<&str>, global_header: bool, body: &str) -> RateLimit {
    let parsed_body: Option<RateLimitBody> = serde_json::from_str(body).ok();

    let retry_after = retry_after_header
        .and_then(|header| header.parse::<f64>().ok())
        .or(parsed_body.as_ref().map(|body| body.retry_after))
        // discord always sends one of the two above in practice; a bare
        // second is a conservative fallback if it ever doesn't
        .unwrap_or(1.0)
        .max(0.0);

    let global = global_header || parsed_body.is_some_and(|body| body.global);

    RateLimit {
        retry_after: Duration::from_secs_f64(retry_after),
        global,
    }
}

/// Parses a rate limit out of a `429` response.
async fn from_response(response: Response) -> RateLimit {
    let retry_after_header = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let global_header = response.headers().contains_key("x-ratelimit-global");

    let body = response.text().await.unwrap_or_default();

    parse_parts(retry_after_header.as_deref(), global_header, &body)
}

/// A request ultimately failed: either a transport-level error, or discord
/// kept rate limiting it past `MAX_RETRIES` (or asked for a wait longer than
/// `MAX_SLEEP`).
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error("discord is rate limiting us; try again in {retry_after:?} :<")]
    RateLimited { retry_after: Duration, global: bool },
}

/// Sends `request`, retrying with backoff if discord responds `429`.
///
/// Returns the first non-`429` response as-is -- callers still need to
/// check `.status()` themselves for other error statuses (`403`, `404`,
/// etc), same as before this existed. Only rate limiting is retried here.
pub async fn send_with_retries(request: RequestBuilder) -> Result<Response, SendError> {
    let mut attempt: u32 = 0;

    loop {
        let attempt_request = request
            .try_clone()
            .expect("discord requests are built from cloneable bodies (get/form/json)");
        let response = attempt_request.send().await?;

        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }

        let rate_limit = from_response(response).await;

        if attempt >= MAX_RETRIES || rate_limit.retry_after > MAX_SLEEP {
            return Err(SendError::RateLimited {
                retry_after: rate_limit.retry_after,
                global: rate_limit.global,
            });
        }

        // exponential backoff as a floor, in case discord's retry_after is
        // suspiciously small (or zero) for some reason
        let backoff = Duration::from_millis(250 * 2u64.pow(attempt));
        tokio::time::sleep(rate_limit.retry_after.max(backoff)).await;

        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_retry_after_header() {
        let rate_limit = parse_parts(Some("2.5"), false, "");
        assert_eq!(rate_limit.retry_after, Duration::from_secs_f64(2.5));
        assert!(!rate_limit.global);
    }

    #[test]
    fn falls_back_to_the_body_when_the_header_is_missing() {
        let rate_limit = parse_parts(None, false, r#"{"retry_after": 0.75, "global": false}"#);
        assert_eq!(rate_limit.retry_after, Duration::from_secs_f64(0.75));
        assert!(!rate_limit.global);
    }

    #[test]
    fn falls_back_to_the_body_when_the_header_is_unparsable() {
        let rate_limit = parse_parts(Some("not-a-number"), false, r#"{"retry_after": 1.2}"#);
        assert_eq!(rate_limit.retry_after, Duration::from_secs_f64(1.2));
    }

    #[test]
    fn reads_global_from_either_the_header_or_the_body() {
        assert!(parse_parts(Some("1"), true, "").global);
        assert!(parse_parts(None, false, r#"{"retry_after": 1, "global": true}"#).global);
    }

    #[test]
    fn defaults_when_nothing_parses() {
        let rate_limit = parse_parts(None, false, "not json");
        assert_eq!(rate_limit.retry_after, Duration::from_secs(1));
        assert!(!rate_limit.global);
    }
}
