// webhooks.rs: the forge webhook endpoint -- verifies a delivery actually
// came from github, normalizes it, checks it against whichever repository
// owners have configured a trigger for, and hands off to the autonomous
// pipeline. Returns 202 immediately and does all the real work in a
// spawned task with its own tracing span, per docs/tracing.md -- github
// retries a slow webhook response as a failed delivery, and none of this
// work needs to finish before the http response does.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Extension, Router,
    body::Bytes,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use munibot_github::{issue_text, normalize_webhook, verify_signature};
use munibot_vcs::{ForgeEvent, RepoTriggerConfig};
use tracing::Instrument;

/// Starts a pipeline run for one matched trigger.
///
/// A trait, rather than a concrete pipeline type, because the autonomous
/// pipeline itself (`munibot_ai::pipeline`) doesn't exist yet -- this is
/// the seam it plugs into once it does, the same way `munibot_ai::Ai` is
/// an `Option` injected into this same server rather than something
/// `server.rs` constructs itself.
#[async_trait]
pub trait PipelineDispatch: Send + Sync {
    async fn dispatch(&self, event: ForgeEvent);
}

/// Everything the webhook route needs beyond the request itself: the
/// shared secret github signs deliveries with, munibot's own bot login (to
/// filter out its own comments), which repositories are configured to
/// trigger a run and how, and where to send a match.
///
/// `webhook_secret` and `dispatch` are both `Option`: a deployment with no
/// `GITHUB_WEBHOOK_SECRET` configured, or no pipeline registered yet, is a
/// deployment where this feature is simply off, not a startup failure --
/// matching the `ai.enabled` convention `server::run`'s own `ai` parameter
/// already establishes.
pub struct WebhookConfig {
    pub webhook_secret: Option<String>,
    pub bot_login: String,
    pub triggers: Vec<RepoTriggerConfig>,
    pub dispatch: Option<Arc<dyn PipelineDispatch>>,
}

/// The one route this module adds: `POST /webhooks/github`.
pub fn router() -> Router {
    Router::new().route("/webhooks/github", post(handle_webhook))
}

async fn handle_webhook(
    Extension(config): Extension<Arc<WebhookConfig>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let span = tracing::info_span!(
        "github_webhook",
        event_type = event_type.as_deref().unwrap_or("unknown")
    );
    tokio::spawn(process_delivery(config, body, event_type, signature).instrument(span));

    // accepted regardless of what processing finds -- a misconfigured or
    // uninteresting delivery is not a delivery failure, and github retries
    // a non-2xx response as though the delivery itself never arrived
    StatusCode::ACCEPTED
}

async fn process_delivery(
    config: Arc<WebhookConfig>,
    body: Bytes,
    event_type: Option<String>,
    signature: Option<String>,
) {
    let Some(webhook_secret) = &config.webhook_secret else {
        tracing::debug!("received a github webhook delivery, but no webhook secret is configured");
        return;
    };

    if let Err(error) = verify_signature(webhook_secret, &body, signature.as_deref()) {
        tracing::warn!(%error, "rejected a github webhook delivery with an invalid signature");
        return;
    }

    let Some(event_type) = event_type else {
        tracing::warn!("github webhook delivery carried no X-GitHub-Event header");
        return;
    };

    let event = match normalize_webhook(&event_type, &body, &config.bot_login) {
        Ok(Some(event)) => event,
        // not an event type or action munibot acts on, or authored by
        // munibot's own identity -- both are silently uninteresting, not
        // failures
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "couldn't normalize a github webhook payload");
            return;
        }
    };

    let Some(trigger) = config
        .triggers
        .iter()
        .find(|trigger| trigger.enabled && trigger.repo == event.issue().repo)
    else {
        return;
    };

    let (title, body_text) = issue_text(&event_type, &body).unwrap_or_default();
    if !trigger.mode.matches(&event, &title, &body_text) {
        return;
    }

    let Some(dispatch) = &config.dispatch else {
        tracing::info!(
            issue = %event.issue(),
            "a trigger matched, but no pipeline is registered on this deployment yet"
        );
        return;
    };

    dispatch.dispatch(event).await;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        body::Body,
        http::{Request, header},
    };
    use munibot_vcs::{Forge, RepoRef, TriggerMode};
    use tower::ServiceExt;

    use super::*;

    const SECRET: &str = "it's a secret to everybody";
    const ISSUE_OPENED: &str = r#"{
        "action": "opened",
        "issue": { "number": 42, "title": "please help", "body": "" },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "someone" }
    }"#;

    struct RecordingDispatch {
        events: Mutex<Vec<ForgeEvent>>,
    }

    #[async_trait]
    impl PipelineDispatch for RecordingDispatch {
        async fn dispatch(&self, event: ForgeEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn signed_body(secret: &str, body: &str) -> (Bytes, String) {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let signature = format!("sha256={}", hex_encode(&mac.finalize().into_bytes()));
        (Bytes::from(body.to_string()), signature)
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn app(config: WebhookConfig) -> Router {
        router().layer(Extension(Arc::new(config)))
    }

    fn request(event_type: &str, body: Bytes, signature: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/webhooks/github")
            .header("X-GitHub-Event", event_type)
            .header("X-Hub-Signature-256", signature)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn test_always_returns_202_even_when_nothing_matches() {
        let config = WebhookConfig {
            webhook_secret: None,
            bot_login: "munibot[bot]".to_string(),
            triggers: vec![],
            dispatch: None,
        };
        let response = app(config)
            .oneshot(request(
                "issues",
                Bytes::from_static(b"{}"),
                "sha256=whatever",
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn test_dispatches_when_a_trigger_matches() {
        let (body, signature) = signed_body(SECRET, ISSUE_OPENED);
        let dispatch = Arc::new(RecordingDispatch {
            events: Mutex::new(vec![]),
        });
        let config = WebhookConfig {
            webhook_secret: Some(SECRET.to_string()),
            bot_login: "munibot[bot]".to_string(),
            triggers: vec![RepoTriggerConfig {
                repo: RepoRef::new(Forge::GitHub, "musicaloft", "munibot"),
                mode: TriggerMode::AllIssues,
                enabled: true,
            }],
            dispatch: Some(dispatch.clone()),
        };

        let response = app(config)
            .oneshot(request("issues", body, &signature))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        // the dispatch itself runs in a spawned task -- give it a moment
        for _ in 0..50 {
            if !dispatch.events.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let events = dispatch.events.lock().unwrap();
        assert_eq!(
            events.len(),
            1,
            "the matching trigger should have dispatched exactly once"
        );
    }

    #[tokio::test]
    async fn test_does_not_dispatch_when_no_repo_is_configured() {
        let (body, signature) = signed_body(SECRET, ISSUE_OPENED);
        let dispatch = Arc::new(RecordingDispatch {
            events: Mutex::new(vec![]),
        });
        let config = WebhookConfig {
            webhook_secret: Some(SECRET.to_string()),
            bot_login: "munibot[bot]".to_string(),
            triggers: vec![],
            dispatch: Some(dispatch.clone()),
        };

        let response = app(config)
            .oneshot(request("issues", body, &signature))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(dispatch.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_does_not_dispatch_with_an_invalid_signature() {
        let dispatch = Arc::new(RecordingDispatch {
            events: Mutex::new(vec![]),
        });
        let config = WebhookConfig {
            webhook_secret: Some(SECRET.to_string()),
            bot_login: "munibot[bot]".to_string(),
            triggers: vec![RepoTriggerConfig {
                repo: RepoRef::new(Forge::GitHub, "musicaloft", "munibot"),
                mode: TriggerMode::AllIssues,
                enabled: true,
            }],
            dispatch: Some(dispatch.clone()),
        };

        let response = app(config)
            .oneshot(request(
                "issues",
                Bytes::from_static(ISSUE_OPENED.as_bytes()),
                "sha256=0000000000000000000000000000000000000000000000000000000000000000",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(dispatch.events.lock().unwrap().is_empty());
    }
}
