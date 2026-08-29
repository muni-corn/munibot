//! Parsing a webhook delivery's `X-GitHub-Event` header and JSON body into a
//! [`ForgeEvent`].

use chrono::{DateTime, Utc};
use munibot_vcs::{Comment, Forge, ForgeEvent, IssueRef, RepoRef};
use serde::Deserialize;

use crate::error::GitHubError;

#[derive(Deserialize)]
struct OwnerPayload {
    login: String,
}

#[derive(Deserialize)]
struct RepositoryPayload {
    name: String,
    owner: OwnerPayload,
}

#[derive(Deserialize)]
struct UserPayload {
    login: String,
}

#[derive(Deserialize)]
struct LabelPayload {
    name: String,
}

#[derive(Deserialize)]
struct IssueLikePayload {
    number: u64,
}

#[derive(Deserialize)]
struct IssuesEventPayload {
    action: String,
    issue: IssueLikePayload,
    label: Option<LabelPayload>,
    repository: RepositoryPayload,
    sender: UserPayload,
}

#[derive(Deserialize)]
struct IssueCommentPayload {
    user: UserPayload,
    body: String,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct IssueCommentEventPayload {
    action: String,
    issue: IssueLikePayload,
    comment: IssueCommentPayload,
    repository: RepositoryPayload,
    sender: UserPayload,
}

#[derive(Deserialize)]
struct ReviewPayload {
    user: UserPayload,
    #[serde(default)]
    body: Option<String>,
    submitted_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct PullRequestReviewEventPayload {
    action: String,
    pull_request: IssueLikePayload,
    review: ReviewPayload,
    repository: RepositoryPayload,
    sender: UserPayload,
}

fn repo_ref(repository: &RepositoryPayload) -> RepoRef {
    RepoRef::new(
        Forge::GitHub,
        repository.owner.login.clone(),
        repository.name.clone(),
    )
}

/// Whether `login` is munibot's own GitHub identity, case-insensitively --
/// GitHub App bot logins carry a `[bot]` suffix and forges are otherwise
/// inconsistent about casing, so an exact byte comparison would be a bug
/// waiting to reopen the infinite comment loop this check exists to avoid.
fn is_self(login: &str, bot_login: &str) -> bool {
    login.eq_ignore_ascii_case(bot_login)
}

/// Parses one webhook delivery into a [`ForgeEvent`], given the
/// `X-GitHub-Event` header's value and the request's raw body.
///
/// Returns `Ok(None)` -- never an error -- for an event type munibot does
/// not act on, an action within a handled event type munibot does not act
/// on (a label removed, an issue closed, a comment edited), or any event
/// authored by `bot_login` itself. The last of these is what keeps munibot
/// from replying to its own comments and triggering itself forever; the
/// first two are simply not interesting yet, not a payload munibot failed
/// to understand.
pub fn normalize_webhook(
    event_type: &str,
    raw_body: &[u8],
    bot_login: &str,
) -> Result<Option<ForgeEvent>, GitHubError> {
    match event_type {
        "issues" => normalize_issues(raw_body, bot_login),
        "issue_comment" => normalize_issue_comment(raw_body, bot_login),
        "pull_request_review" => normalize_pull_request_review(raw_body, bot_login),
        _ => Ok(None),
    }
}

fn normalize_issues(raw_body: &[u8], bot_login: &str) -> Result<Option<ForgeEvent>, GitHubError> {
    let payload: IssuesEventPayload = serde_json::from_slice(raw_body)
        .map_err(|error| GitHubError::Payload(error.to_string()))?;

    if is_self(&payload.sender.login, bot_login) {
        return Ok(None);
    }

    let issue = IssueRef::new(repo_ref(&payload.repository), payload.issue.number);

    match payload.action.as_str() {
        "opened" => Ok(Some(ForgeEvent::IssueOpened { issue })),
        "labeled" => match payload.label {
            Some(label) => Ok(Some(ForgeEvent::IssueLabeled {
                issue,
                label: label.name,
            })),
            // github's own docs guarantee `label` on a labeled action; a
            // payload missing it is malformed, not merely uninteresting
            None => Err(GitHubError::Payload(
                "issues event with action \"labeled\" is missing its label".to_string(),
            )),
        },
        _ => Ok(None),
    }
}

fn normalize_issue_comment(
    raw_body: &[u8],
    bot_login: &str,
) -> Result<Option<ForgeEvent>, GitHubError> {
    let payload: IssueCommentEventPayload = serde_json::from_slice(raw_body)
        .map_err(|error| GitHubError::Payload(error.to_string()))?;

    if payload.action != "created" || is_self(&payload.sender.login, bot_login) {
        return Ok(None);
    }

    let issue = IssueRef::new(repo_ref(&payload.repository), payload.issue.number);
    Ok(Some(ForgeEvent::IssueCommented {
        issue,
        comment: Comment {
            author: payload.comment.user.login,
            body: payload.comment.body,
            created_at: payload.comment.created_at,
        },
    }))
}

#[derive(Deserialize)]
struct IssueTextPayload {
    title: String,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Deserialize)]
struct IssuesEventTextPayload {
    issue: IssueTextPayload,
}

/// The issue's own title and body, for [`munibot_vcs::TriggerMode::Keyword`]
/// matching -- which needs real text, unlike a [`ForgeEvent`], which only
/// ever carries a bare [`IssueRef`].
///
/// Only the `"issues"` event carries the issue's own title and body
/// directly in its payload; every other event type this crate normalizes
/// describes something that *happened on* an issue (a comment, a review)
/// without repeating the issue's own text, so this returns `None` for
/// those rather than a call that always fails.
pub fn issue_text(event_type: &str, raw_body: &[u8]) -> Option<(String, String)> {
    if event_type != "issues" {
        return None;
    }

    let payload: IssuesEventTextPayload = serde_json::from_slice(raw_body).ok()?;
    Some((payload.issue.title, payload.issue.body.unwrap_or_default()))
}

fn normalize_pull_request_review(
    raw_body: &[u8],
    bot_login: &str,
) -> Result<Option<ForgeEvent>, GitHubError> {
    let payload: PullRequestReviewEventPayload = serde_json::from_slice(raw_body)
        .map_err(|error| GitHubError::Payload(error.to_string()))?;

    if payload.action != "submitted" || is_self(&payload.sender.login, bot_login) {
        return Ok(None);
    }

    let issue = IssueRef::new(repo_ref(&payload.repository), payload.pull_request.number);
    Ok(Some(ForgeEvent::PullRequestReviewed {
        issue,
        comment: Comment {
            author: payload.review.user.login,
            body: payload.review.body.unwrap_or_default(),
            created_at: payload.review.submitted_at,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE_OPENED: &str = r#"{
        "action": "opened",
        "issue": { "number": 42 },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "someone" }
    }"#;

    const ISSUE_LABELED: &str = r#"{
        "action": "labeled",
        "issue": { "number": 42 },
        "label": { "name": "ai-triage" },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "a-maintainer" }
    }"#;

    const ISSUE_CLOSED: &str = r#"{
        "action": "closed",
        "issue": { "number": 42 },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "someone" }
    }"#;

    const ISSUE_COMMENT_CREATED: &str = r#"{
        "action": "created",
        "issue": { "number": 42 },
        "comment": {
            "user": { "login": "someone" },
            "body": "can you share a stack trace?",
            "created_at": "2026-01-01T00:00:00Z"
        },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "someone" }
    }"#;

    const PULL_REQUEST_REVIEW_SUBMITTED: &str = r#"{
        "action": "submitted",
        "pull_request": { "number": 7 },
        "review": {
            "user": { "login": "a-maintainer" },
            "body": "looks good, one nit",
            "submitted_at": "2026-01-01T00:00:00Z"
        },
        "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
        "sender": { "login": "a-maintainer" }
    }"#;

    #[test]
    fn test_ignores_an_event_type_munibot_does_not_act_on() {
        let result = normalize_webhook("star", b"{}", "munibot[bot]").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_normalizes_an_issue_opened_event() {
        let event = normalize_webhook("issues", ISSUE_OPENED.as_bytes(), "munibot[bot]")
            .unwrap()
            .expect("should normalize");

        match event {
            ForgeEvent::IssueOpened { issue } => {
                assert_eq!(issue.number, 42);
                assert_eq!(issue.repo.owner, "musicaloft");
                assert_eq!(issue.repo.name, "munibot");
            }
            other => panic!("expected IssueOpened, got {other:?}"),
        }
    }

    #[test]
    fn test_normalizes_an_issue_labeled_event() {
        let event = normalize_webhook("issues", ISSUE_LABELED.as_bytes(), "munibot[bot]")
            .unwrap()
            .expect("should normalize");

        match event {
            ForgeEvent::IssueLabeled { label, .. } => assert_eq!(label, "ai-triage"),
            other => panic!("expected IssueLabeled, got {other:?}"),
        }
    }

    #[test]
    fn test_ignores_an_issues_action_munibot_does_not_act_on() {
        let result = normalize_webhook("issues", ISSUE_CLOSED.as_bytes(), "munibot[bot]").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_normalizes_an_issue_comment_created_event() {
        let event = normalize_webhook(
            "issue_comment",
            ISSUE_COMMENT_CREATED.as_bytes(),
            "munibot[bot]",
        )
        .unwrap()
        .expect("should normalize");

        match event {
            ForgeEvent::IssueCommented { comment, .. } => {
                assert_eq!(comment.body, "can you share a stack trace?");
            }
            other => panic!("expected IssueCommented, got {other:?}"),
        }
    }

    #[test]
    fn test_normalizes_a_pull_request_review_submitted_event() {
        let event = normalize_webhook(
            "pull_request_review",
            PULL_REQUEST_REVIEW_SUBMITTED.as_bytes(),
            "munibot[bot]",
        )
        .unwrap()
        .expect("should normalize");

        match event {
            ForgeEvent::PullRequestReviewed { issue, comment } => {
                assert_eq!(issue.number, 7);
                assert_eq!(comment.body, "looks good, one nit");
            }
            other => panic!("expected PullRequestReviewed, got {other:?}"),
        }
    }

    #[test]
    fn test_filters_out_an_issue_opened_by_munibot_itself() {
        let result = normalize_webhook("issues", ISSUE_OPENED.as_bytes(), "someone").unwrap();
        assert!(
            result.is_none(),
            "an event authored by munibot's own identity must never re-enter the pipeline"
        );
    }

    #[test]
    fn test_filters_out_a_comment_posted_by_munibot_itself() {
        let result =
            normalize_webhook("issue_comment", ISSUE_COMMENT_CREATED.as_bytes(), "someone")
                .unwrap();
        assert!(
            result.is_none(),
            "replying to its own comment would trigger munibot forever"
        );
    }

    #[test]
    fn test_self_filtering_is_case_insensitive() {
        let result = normalize_webhook("issues", ISSUE_OPENED.as_bytes(), "SOMEONE").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_a_labeled_action_missing_its_label_is_a_payload_error() {
        let malformed = r#"{
            "action": "labeled",
            "issue": { "number": 42 },
            "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
            "sender": { "login": "someone" }
        }"#;

        let error = normalize_webhook("issues", malformed.as_bytes(), "munibot[bot]")
            .expect_err("a labeled action with no label is malformed, not merely ignorable");
        assert!(matches!(error, GitHubError::Payload(_)));
    }

    #[test]
    fn test_malformed_json_is_a_payload_error() {
        let error = normalize_webhook("issues", b"not json at all", "munibot[bot]")
            .expect_err("garbage json should be a payload error");
        assert!(matches!(error, GitHubError::Payload(_)));
    }

    #[test]
    fn test_issue_text_extracts_title_and_body_from_an_issues_event() {
        let body = r#"{
            "action": "opened",
            "issue": { "number": 1, "title": "it crashes", "body": "steps to reproduce" },
            "repository": { "name": "munibot", "owner": { "login": "musicaloft" } },
            "sender": { "login": "someone" }
        }"#;

        let (title, issue_body) = issue_text("issues", body.as_bytes()).expect("should extract");
        assert_eq!(title, "it crashes");
        assert_eq!(issue_body, "steps to reproduce");
    }

    #[test]
    fn test_issue_text_returns_none_for_other_event_types() {
        assert_eq!(issue_text("issue_comment", b"{}"), None);
        assert_eq!(issue_text("pull_request_review", b"{}"), None);
    }
}
