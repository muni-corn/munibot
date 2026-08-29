//! `GitHubForge`: the concrete implementation of `munibot_vcs`'s
//! forge-agnostic traits over an installation-authenticated `octocrab`
//! client.

use std::path::Path;

use async_trait::async_trait;
use http::StatusCode;
use munibot_vcs::{
    Comment, Issue, IssueRef, IssueSource, IssueState, PullRequestTarget, RepoRef, VcsError,
};
use octocrab::{
    Octocrab,
    models::{self, AppId, InstallationId, repos::Object},
    params::repos::Reference,
};
use tokio::process::Command;
use url::Url;

use crate::auth::{InstallationTokenCache, OctocrabTokenMinter};

/// Authenticates once as a GitHub App installation and implements both
/// [`IssueSource`] and [`PullRequestTarget`] over that same installation.
pub struct GitHubForge {
    installation_id: InstallationId,
    client: Octocrab,
    tokens: InstallationTokenCache,
}

impl GitHubForge {
    /// Builds a forge scoped to one installation. `private_key` is the
    /// PEM-encoded contents of `GITHUB_APP_PRIVATE_KEY`.
    pub fn new(
        app_id: AppId,
        private_key: &str,
        installation_id: InstallationId,
    ) -> Result<Self, crate::GitHubError> {
        let minter = OctocrabTokenMinter::new(app_id, private_key)?;
        let client = minter.installation_client(installation_id)?;
        Ok(Self {
            installation_id,
            client,
            tokens: InstallationTokenCache::new(minter),
        })
    }
}

/// Classifies an octocrab-reported GitHub API error into the forge-agnostic
/// shape [`VcsError`] itself defines.
///
/// A free function over just the inner [`octocrab::GitHubError`] (status
/// code and message), rather than the outer [`octocrab::Error`] enum,
/// specifically so it stays unit-testable: the outer enum's `GitHub`
/// variant carries a `snafu::Backtrace` that is awkward to construct by
/// hand in a test, while the inner struct is a plain, publicly
/// constructible value.
fn classify_github_error(status_code: StatusCode, message: &str) -> VcsError {
    match status_code {
        StatusCode::NOT_FOUND => VcsError::NotFound(message.to_string()),
        StatusCode::FORBIDDEN => VcsError::Forbidden(message.to_string()),
        StatusCode::TOO_MANY_REQUESTS => VcsError::RateLimited,
        StatusCode::UNAUTHORIZED => VcsError::Authentication(message.to_string()),
        _ => VcsError::Other(message.to_string()),
    }
}

fn map_error(error: octocrab::Error) -> VcsError {
    match error {
        octocrab::Error::GitHub { source, .. } => {
            classify_github_error(source.status_code, &source.message)
        }
        other => VcsError::Other(other.to_string()),
    }
}

/// Builds an HTTPS clone URL with the token embedded in the userinfo
/// component, the shape github's own credential-helper-free git access
/// expects (`https://x-access-token:<token>@github.com/owner/name.git`).
///
/// A pure function, tested directly with a fake token, so the real
/// [`GitHubForge::clone_url_with_token`] never needs a live token to
/// exercise this part of its own behaviour.
fn build_clone_url(owner: &str, name: &str, token: &str) -> Result<Url, VcsError> {
    Url::parse(&format!(
        "https://x-access-token:{token}@github.com/{owner}/{name}.git"
    ))
    .map_err(|error| VcsError::Other(format!("couldn't build a clone url: {error}")))
}

#[async_trait]
impl IssueSource for GitHubForge {
    async fn fetch_issue(&self, issue: &IssueRef) -> Result<Issue, VcsError> {
        let handler = self.client.issues(&issue.repo.owner, &issue.repo.name);
        let fetched = handler.get(issue.number).await.map_err(map_error)?;

        Ok(Issue {
            reference: issue.clone(),
            title: fetched.title,
            body: fetched.body.unwrap_or_default(),
            author: fetched.user.login,
            labels: fetched.labels.into_iter().map(|label| label.name).collect(),
            state: match fetched.state {
                models::IssueState::Open => IssueState::Open,
                models::IssueState::Closed => IssueState::Closed,
                // github's own model is non-exhaustive for forward
                // compatibility; an issue in a state neither open nor
                // closed is still, as far as munibot is concerned, not open
                _ => IssueState::Closed,
            },
        })
    }

    async fn list_comments(&self, issue: &IssueRef) -> Result<Vec<Comment>, VcsError> {
        let handler = self.client.issues(&issue.repo.owner, &issue.repo.name);
        let page = handler
            .list_comments(issue.number)
            .send()
            .await
            .map_err(map_error)?;

        Ok(page
            .items
            .into_iter()
            .map(|comment| Comment {
                author: comment.user.login,
                body: comment.body.unwrap_or_default(),
                created_at: comment.created_at,
            })
            .collect())
    }

    async fn post_comment(&self, issue: &IssueRef, body: &str) -> Result<(), VcsError> {
        let handler = self.client.issues(&issue.repo.owner, &issue.repo.name);
        handler
            .create_comment(issue.number, body)
            .await
            .map_err(map_error)?;
        Ok(())
    }
}

#[async_trait]
impl PullRequestTarget for GitHubForge {
    async fn create_branch(
        &self,
        repo: &RepoRef,
        branch: &str,
        base: &str,
    ) -> Result<(), VcsError> {
        let handler = self.client.repos(&repo.owner, &repo.name);
        let base_ref = handler
            .get_ref(&Reference::Branch(base.to_string()))
            .await
            .map_err(map_error)?;

        let sha = match base_ref.object {
            Object::Commit { sha, .. } | Object::Tag { sha, .. } => sha,
            // github's own model is non-exhaustive for forward
            // compatibility with ref types it might add later
            _ => {
                return Err(VcsError::Other(
                    "base branch ref did not resolve to a commit or tag".to_string(),
                ));
            }
        };

        match handler
            .create_ref(&Reference::Branch(branch.to_string()), sha)
            .await
        {
            Ok(_) => Ok(()),
            // idempotent: the pipeline reuses a branch across a rejected
            // plan's retry (see the branch naming commit), so a branch
            // that already exists is success, not failure
            Err(octocrab::Error::GitHub { source, .. })
                if source.status_code == StatusCode::UNPROCESSABLE_ENTITY
                    && source.message.to_lowercase().contains("already exists") =>
            {
                Ok(())
            }
            Err(error) => Err(map_error(error)),
        }
    }

    async fn push(&self, repo: &RepoRef, branch: &str, working_dir: &Path) -> Result<(), VcsError> {
        let url = self.clone_url_with_token(repo).await?;

        let status = Command::new("git")
            .arg("-C")
            .arg(working_dir)
            .arg("push")
            .arg(url.as_str())
            .arg(format!("HEAD:refs/heads/{branch}"))
            .status()
            .await
            .map_err(|error| VcsError::Other(format!("couldn't run git push: {error}")))?;

        if status.success() {
            Ok(())
        } else {
            // never include the url (or anything derived from the command
            // line) in this error -- it carries a live installation token
            Err(VcsError::Other(format!("git push exited with {status}")))
        }
    }

    async fn open_pull_request(
        &self,
        repo: &RepoRef,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<IssueRef, VcsError> {
        let handler = self.client.pulls(&repo.owner, &repo.name);
        let pull_request = handler
            .create(title, head, base)
            .body(body)
            .send()
            .await
            .map_err(map_error)?;

        Ok(IssueRef::new(repo.clone(), pull_request.number))
    }

    async fn clone_url_with_token(&self, repo: &RepoRef) -> Result<Url, VcsError> {
        let token = self
            .tokens
            .token_for(self.installation_id)
            .await
            .map_err(|error| VcsError::Authentication(error.to_string()))?;
        build_clone_url(&repo.owner, &repo.name, &token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_github_error_maps_404_to_not_found() {
        assert!(matches!(
            classify_github_error(StatusCode::NOT_FOUND, "Not Found"),
            VcsError::NotFound(_)
        ));
    }

    #[test]
    fn test_classify_github_error_maps_403_to_forbidden() {
        assert!(matches!(
            classify_github_error(StatusCode::FORBIDDEN, "no access"),
            VcsError::Forbidden(_)
        ));
    }

    #[test]
    fn test_classify_github_error_maps_429_to_rate_limited() {
        assert!(matches!(
            classify_github_error(StatusCode::TOO_MANY_REQUESTS, "slow down"),
            VcsError::RateLimited
        ));
    }

    #[test]
    fn test_classify_github_error_maps_401_to_authentication() {
        assert!(matches!(
            classify_github_error(StatusCode::UNAUTHORIZED, "bad credentials"),
            VcsError::Authentication(_)
        ));
    }

    #[test]
    fn test_classify_github_error_maps_everything_else_to_other() {
        assert!(matches!(
            classify_github_error(StatusCode::INTERNAL_SERVER_ERROR, "server exploded"),
            VcsError::Other(_)
        ));
    }

    #[test]
    fn test_build_clone_url_embeds_the_token_as_userinfo() {
        let url = build_clone_url("musicaloft", "munibot", "ghs_faketoken").unwrap();
        assert_eq!(
            url.as_str(),
            "https://x-access-token:ghs_faketoken@github.com/musicaloft/munibot.git"
        );
    }
}
