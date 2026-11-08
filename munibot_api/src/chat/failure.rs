use std::time::SystemTime;

use munibot_ai::rig::ProviderError;

/// Says what went wrong with a reply, in words the person chatting can act
/// on.
///
/// rig's own error text is the provider's raw reply, which is unreadable at
/// best and leaks request details at worst, so it is logged server-side and
/// never shown. This only reads rig's classification of the failure: its
/// status code, whether it's worth retrying, and how long the provider asked
/// us to wait.
pub(crate) fn describe_failure(error: &ProviderError) -> String {
    let report = error.report();
    let status = error
        .provider_response_status()
        .map(|status| status.as_u16());

    if report.is_context_overflow() {
        return "this conversation has gotten too long for munibot to read all at once :< start a \
                new one to keep chatting"
            .to_string();
    }

    match status {
        Some(429) => match report.retry_after(SystemTime::now()) {
            Some(wait) if wait.as_secs() > 0 => format!(
                "munibot's model provider is busy right now :< try again in about {} seconds",
                wait.as_secs()
            ),
            _ => "munibot's model provider is busy right now :< try again in a moment".to_string(),
        },
        Some(401 | 403) => "munibot can't reach his model right now :< an operator needs to check \
                            its api key"
            .to_string(),
        _ if error.is_retryable() => {
            "munibot's model provider had a hiccup :< try again in a moment".to_string()
        }
        _ => "something went wrong talking to munibot's model :< try again, and let an operator \
              know if it keeps happening"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::StatusCode;

    use super::*;

    fn response(status: StatusCode) -> ProviderError {
        ProviderError::from_http_response(status, "{}")
    }

    #[test]
    fn test_a_rate_limit_asks_to_try_again() {
        let message = describe_failure(&response(StatusCode::TOO_MANY_REQUESTS));
        assert!(message.contains("busy"), "{message}");
    }

    #[test]
    fn test_a_rejected_key_points_at_an_operator() {
        let message = describe_failure(&response(StatusCode::UNAUTHORIZED));
        assert!(message.contains("api key"), "{message}");
    }

    #[test]
    fn test_a_server_error_is_a_retryable_hiccup() {
        let message = describe_failure(&response(StatusCode::BAD_GATEWAY));
        assert!(message.contains("hiccup"), "{message}");
    }

    #[test]
    fn test_the_providers_raw_reply_is_never_shown() {
        let error = ProviderError::from_http_response(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"secret request detail"}}"#,
        );
        let message = describe_failure(&error);
        assert!(!message.contains("secret request detail"), "{message}");
    }
}
