//! Classifies rig's completion errors into ours, so the retry layer knows what
//! is worth retrying.
//!
//! rig's error model carries an HTTP status when one is available (via the
//! public `provider_response_status()` helper generated on
//! [`CompletionError`]), but never the response headers - so a `Retry-After`
//! value can never be extracted here. This is a fourth rig API gap
//! alongside the three already recorded in
//! `docs/notes/ai-preflight-findings.md`: no `DynClientBuilder`, a
//! non-object-safe `CompletionModel`, and no normalized stop reason. A missing
//! `retry_after` is not a bug in this function; the retry policy simply falls
//! back to its own computed backoff whenever the provider gives no better
//! guidance.

use rig_core::completion::CompletionError;

use crate::types::AiError;

/// Classifies a rig completion error, choosing between a transient
/// [`AiError::Provider`], a permanent [`AiError::Rejected`], and
/// [`AiError::RateLimited`] based on the HTTP status the error carries, when it
/// carries one at all.
pub fn classify_completion_error(error: CompletionError) -> AiError {
    let status = error.provider_response_status();
    let message = error.to_string();

    match status {
        Some(status) if status.as_u16() == 429 => AiError::RateLimited { retry_after: None },
        Some(status) if status.is_server_error() => AiError::Provider(message),
        Some(status) if status.is_client_error() => AiError::Rejected(message),
        // no status at all (a connection error, a timeout, a malformed JSON body): treat as
        // transient, since these are exactly the class of failure a retry is likely to outlive
        _ => AiError::Provider(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_status_error(status: u16) -> CompletionError {
        CompletionError::from_http_response(
            http::StatusCode::from_u16(status).expect("valid status code"),
            "provider said no",
        )
    }

    #[test]
    fn test_429_is_classified_as_rate_limited() {
        let error = classify_completion_error(http_status_error(429));
        assert!(
            matches!(error, AiError::RateLimited { retry_after: None }),
            "429 should be rate limited, with no retry_after since rig carries no headers"
        );
        assert!(
            error.is_transient(),
            "a rate limit should be worth retrying"
        );
    }

    #[test]
    fn test_5xx_is_classified_as_transient_provider_trouble() {
        for status in [500, 502, 503, 504] {
            let error = classify_completion_error(http_status_error(status));
            assert!(
                matches!(error, AiError::Provider(_)),
                "status {status} should be provider trouble"
            );
            assert!(
                error.is_transient(),
                "status {status} should be worth retrying"
            );
        }
    }

    #[test]
    fn test_4xx_other_than_429_is_classified_as_rejected() {
        for status in [400, 401, 403, 404, 422] {
            let error = classify_completion_error(http_status_error(status));
            assert!(
                matches!(error, AiError::Rejected(_)),
                "status {status} should be a flat rejection, not something to retry"
            );
            assert!(
                !error.is_transient(),
                "status {status} is never worth retrying unchanged"
            );
        }
    }

    #[test]
    fn test_no_status_defaults_to_transient() {
        // a JSON parse failure carries no status at all - connection-level trouble, not
        // a rejection of the request itself, so it defaults to the transient
        // side
        let error =
            classify_completion_error(CompletionError::ResponseError("invalid json".to_string()));
        assert!(
            matches!(error, AiError::Provider(_)),
            "an error with no HTTP status should default to transient provider trouble"
        );
    }

    #[test]
    fn test_success_status_with_a_provider_error_envelope_is_classified() {
        // some providers return HTTP 200 with an error payload in the body;
        // from_http_response routes a success status into ProviderResponse
        // rather than HttpError, and provider_response_status() still surfaces
        // it from there
        let error = CompletionError::from_http_response(
            http::StatusCode::OK,
            "{\"error\": \"actually failed\"}",
        );
        // a 2xx is neither a 4xx nor a 5xx, so this falls through to the transient
        // default - there is no HTTP-level signal here to say otherwise
        assert!(matches!(
            classify_completion_error(error),
            AiError::Provider(_)
        ));
    }
}
