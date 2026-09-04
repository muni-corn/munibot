//! `GitHubForge`'s trait bodies, run against a real HTTP server for the
//! first time - gap 14. Every request `create_branch` and
//! `open_pull_request` make goes through `octocrab`, which
//! `Octocrab::builder().base_uri(...)` points at a `wiremock` server
//! instead of the real GitHub API; `GitHubForge::from_parts` builds a
//! forge over that client directly, bypassing the real App-JWT exchange
//! entirely (already covered by this crate's own `auth` module tests).
//!
//! `octocrab::GitHubError` is `#[non_exhaustive]` with no public
//! constructor - a mocked HTTP server is the only way to actually
//! produce one of the specific error shapes `create_branch`'s own match
//! arm needs to be tested against, rather than a hand-built one that
//! would compile against a different version's own field set.

use std::path::PathBuf;

use async_trait::async_trait;
use munibot_github::{GitHubError, GitHubForge, InstallationTokenCache, TokenMinter};
use munibot_vcs::{Forge, PullRequestTarget, RepoRef};
use octocrab::{Octocrab, models::InstallationId};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// Always mints the same fixed string - real enough for `push`'s own
/// `clone_url_with_token` call, which only ever splices this into a url,
/// never validates it itself.
struct FixedTokenMinter(String);

#[async_trait]
impl TokenMinter for FixedTokenMinter {
    async fn mint(&self, _installation_id: InstallationId) -> Result<String, GitHubError> {
        Ok(self.0.clone())
    }
}

fn octocrab_at(uri: &str) -> Octocrab {
    Octocrab::builder().base_uri(uri).unwrap().build().unwrap()
}

fn forge_at(uri: &str) -> GitHubForge {
    GitHubForge::from_parts(
        InstallationId(1),
        octocrab_at(uri),
        InstallationTokenCache::new(FixedTokenMinter("fake-installation-token".to_string())),
    )
}

fn cocoa_repo() -> RepoRef {
    RepoRef::new(Forge::GitHub, "muni-corn", "cocoa")
}

/// A minimal, real `models::repos::Ref` payload - exactly the fields
/// `get_ref` actually requires, per octocrab's own struct definition.
fn ref_response_body(sha: &str) -> serde_json::Value {
    serde_json::json!({
        "ref": "refs/heads/main",
        "node_id": "abc123",
        "url": "https://api.github.com/repos/muni-corn/cocoa/git/refs/heads/main",
        "object": {
            "type": "commit",
            "sha": sha,
            "url": format!("https://api.github.com/repos/muni-corn/cocoa/git/commits/{sha}"),
        }
    })
}

#[tokio::test]
async fn test_create_branch_reuses_an_already_existing_branch_idempotently() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/muni-corn/cocoa/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ref_response_body("abc123")))
        .mount(&server)
        .await;

    // github's own real response shape for "a ref with this name
    // already exists" - status 422, a plain {"message": ...} body -
    // exactly what create_branch's own match arm looks for
    Mock::given(method("POST"))
        .and(path("/repos/muni-corn/cocoa/git/refs"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "message": "Reference already exists",
            "documentation_url": "https://docs.github.com/rest/git/refs#create-a-reference"
        })))
        .mount(&server)
        .await;

    let forge = forge_at(&server.uri());
    let result = forge
        .create_branch(&cocoa_repo(), "munibot/issue-17", "main")
        .await;

    assert!(
        result.is_ok(),
        "a branch that already exists should be treated as success, got {result:?}"
    );
}

#[tokio::test]
async fn test_create_branch_propagates_a_real_failure() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/repos/muni-corn/cocoa/git/ref/heads/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ref_response_body("abc123")))
        .mount(&server)
        .await;

    // a failure unrelated to "already exists" - a real create_ref
    // failure (a permissions problem, say) - must still propagate as an
    // error, not be swallowed by the same idempotent-reuse arm
    Mock::given(method("POST"))
        .and(path("/repos/muni-corn/cocoa/git/refs"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "Resource not accessible by integration"
        })))
        .mount(&server)
        .await;

    let forge = forge_at(&server.uri());
    let result = forge
        .create_branch(&cocoa_repo(), "munibot/issue-17", "main")
        .await;

    assert!(
        result.is_err(),
        "a real failure creating the ref must not be treated as idempotent reuse"
    );
}

#[tokio::test]
async fn test_open_pull_request_end_to_end() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/repos/muni-corn/cocoa/pulls"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 1,
            "number": 42,
            "url": "https://api.github.com/repos/muni-corn/cocoa/pulls/42",
            "head": { "ref": "munibot/issue-17", "sha": "abc123" },
            "base": { "ref": "main", "sha": "def456" },
        })))
        .mount(&server)
        .await;

    let forge = forge_at(&server.uri());
    let issue = forge
        .open_pull_request(
            &cocoa_repo(),
            "munibot/issue-17",
            "main",
            "fix: the crash",
            "closes #17",
        )
        .await
        .expect("should succeed");

    assert_eq!(issue.number, 42);
    assert_eq!(issue.repo, cocoa_repo());
}

/// `push` shells out to a real `git push`, never through `octocrab` -
/// `clone_url_with_token` always builds a real `github.com` url (never
/// wiremock's own, since it has no reason to be configurable the way
/// `octocrab`'s own base uri is), so this is the one test in this suite
/// that touches the real network - to a repository that does not exist,
/// which is exactly what proves the failure path without needing any
/// push access at all.
#[tokio::test]
async fn test_push_fails_cleanly_against_a_repository_that_does_not_exist() {
    let work_dir = std::env::temp_dir().join(format!(
        "munibot_github_push_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&work_dir).unwrap();
    let dir = work_dir.display().to_string();
    let run = |args: &'static [&'static str]| {
        let dir = dir.clone();
        async move {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .status()
                .await
                .unwrap()
        }
    };
    run(&["init", "--initial-branch", "main"]).await;
    run(&["config", "user.email", "muni@example.com"]).await;
    run(&["config", "user.name", "muni"]).await;
    run(&["config", "commit.gpgsign", "false"]).await;
    std::fs::write(work_dir.join("README.md"), "hello").unwrap();
    run(&["add", "."]).await;
    run(&["commit", "-m", "initial commit"]).await;

    let forge = GitHubForge::from_parts(
        InstallationId(1),
        octocrab_at("https://api.github.com"),
        InstallationTokenCache::new(FixedTokenMinter("not-a-real-token".to_string())),
    );
    let repo = RepoRef::new(
        Forge::GitHub,
        "definitely-does-not-exist-owner-abc123",
        "definitely-does-not-exist-repo-xyz789",
    );

    let result = forge
        .push(&repo, "munibot/test-branch", &PathBuf::from(&work_dir))
        .await;
    assert!(
        result.is_err(),
        "pushing to a nonexistent repository with a fake token should fail"
    );

    std::fs::remove_dir_all(&work_dir).ok();
}
