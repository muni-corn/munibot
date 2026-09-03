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
use munibot_ai::project::ProjectManager;
use munibot_github::{issue_text, normalize_webhook, verify_signature};
use munibot_vcs::{ForgeEvent, RepoRef, TriggerMode};
use tracing::Instrument;

/// Starts a pipeline run for one matched trigger.
///
/// A trait, rather than naming `PipelineLauncher` directly, so this
/// module never has to depend on `munibot_ai::pipeline` or
/// `munibot_github` itself - `PipelineLauncher` is the real
/// implementation (`super::pipeline_launcher`), injected as an `Option`
/// the same way `munibot_ai::Ai` is: `server.rs` doesn't construct
/// either itself, and a deployment with no `GITHUB_APP_ID` configured
/// runs with this `None`, exactly as it did before the pipeline
/// existed at all.
#[async_trait]
pub trait PipelineDispatch: Send + Sync {
    async fn dispatch(&self, event: ForgeEvent);
}

/// Everything the webhook route needs beyond the request itself: the
/// shared secret github signs deliveries with, munibot's own bot login (to
/// filter out its own comments), the project store a delivery's own
/// repository is looked up against (trigger mode and label live on the
/// project row - see `Project::trigger`'s own doc comment - not a TOML
/// section, so the projects page can edit them without a restart), and
/// where to send a match.
///
/// `webhook_secret` and `dispatch` are both `Option`: a deployment with no
/// `GITHUB_WEBHOOK_SECRET` configured, or no pipeline registered yet, is a
/// deployment where this feature is simply off, not a startup failure --
/// matching the `ai.enabled` convention `server::run`'s own `ai` parameter
/// already establishes.
pub struct WebhookConfig {
    pub webhook_secret: Option<String>,
    pub bot_login: String,
    pub project_manager: Arc<ProjectManager>,
    pub dispatch: Option<Arc<dyn PipelineDispatch>>,
}

/// The trigger mode a delivery's own repository is currently configured
/// with, read fresh from the project store on every delivery rather than
/// cached - an operator editing a project's trigger on the projects page
/// must take effect on the very next delivery, not the next restart.
/// `None` for an unregistered repository, or one registered but not yet
/// given a trigger at all.
///
/// A linear scan, the same reasoning
/// `munibot_gui::server::pipeline_launcher::PipelineLauncher::project_for`'s
/// own doc comment gives: the expected project count is small, and a
/// dedicated `get_project_by_repo` query has no caller besides this one
/// and that one yet.
async fn trigger_for(project_manager: &ProjectManager, repo: &RepoRef) -> Option<TriggerMode> {
    match project_manager.list_projects().await {
        Ok(projects) => projects
            .into_iter()
            .find(|project| &project.repo == repo)
            .and_then(|project| project.trigger),
        Err(error) => {
            tracing::warn!(%error, "couldn't look up registered projects for a webhook delivery");
            None
        }
    }
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

    let Some(trigger_mode) = trigger_for(&config.project_manager, &event.issue().repo).await else {
        return;
    };

    let (title, body_text) = issue_text(&event_type, &body).unwrap_or_default();
    if !trigger_mode.matches(&event, &title, &body_text) {
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
    use munibot_ai::project::{InMemoryProjectStore, NewProject, ProjectStore, ProjectsSettings};
    use munibot_vcs::Forge;
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

    fn munibot_repo() -> RepoRef {
        RepoRef::new(Forge::GitHub, "musicaloft", "munibot")
    }

    /// A `ProjectManager` with no registered projects at all - the
    /// "nothing is configured for this repository" case.
    async fn empty_project_manager() -> Arc<ProjectManager> {
        Arc::new(ProjectManager::new(
            Arc::new(InMemoryProjectStore::new()),
            ProjectsSettings::default(),
        ))
    }

    /// A `ProjectManager` with one project registered for
    /// `munibot_repo`, on `trigger` - no real clone at all, since
    /// `webhooks.rs` never reaches anything that would need one.
    async fn project_manager_with_trigger(trigger: Option<TriggerMode>) -> Arc<ProjectManager> {
        let store = Arc::new(InMemoryProjectStore::new());
        store
            .upsert_project(&NewProject {
                slug: "munibot".to_string(),
                display_name: "munibot".to_string(),
                repo: munibot_repo(),
                clone_url: "https://example.invalid/munibot.git".to_string(),
                default_branch: "main".to_string(),
                local_path: std::path::PathBuf::from("/tmp/does-not-matter"),
                image_strategy: munibot_ai::project::ImageStrategy::Base,
                network_policy: munibot_ai::sandbox::config::SandboxNetworkConfig::None,
                trigger,
                installation_id: None,
            })
            .await
            .expect("upsert failed");
        Arc::new(ProjectManager::new(store, ProjectsSettings::default()))
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
            project_manager: empty_project_manager().await,
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
            project_manager: project_manager_with_trigger(Some(TriggerMode::AllIssues)).await,
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
            project_manager: empty_project_manager().await,
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

    /// A project registered for the repository but with no trigger
    /// configured yet (`Project::trigger` is `None`) must behave
    /// exactly like an unregistered one - not dispatch, not panic.
    #[tokio::test]
    async fn test_does_not_dispatch_for_a_registered_project_with_no_trigger_configured() {
        let (body, signature) = signed_body(SECRET, ISSUE_OPENED);
        let dispatch = Arc::new(RecordingDispatch {
            events: Mutex::new(vec![]),
        });
        let config = WebhookConfig {
            webhook_secret: Some(SECRET.to_string()),
            bot_login: "munibot[bot]".to_string(),
            project_manager: project_manager_with_trigger(None).await,
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
            project_manager: project_manager_with_trigger(Some(TriggerMode::AllIssues)).await,
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
