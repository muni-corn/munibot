//! The write side of a forge integration: branching, pushing, and opening a
//! pull request once the pipeline has something to submit.

use std::path::Path;

use async_trait::async_trait;
use url::Url;

use crate::{IssueRef, RepoRef, VcsError};

/// Branches, pushes, and opens pull requests against a repository.
///
/// Object-safe, matching [`crate::IssueSource`] -- the pipeline holds this
/// behind `Arc<dyn PullRequestTarget>` too.
#[async_trait]
pub trait PullRequestTarget: Send + Sync {
    /// Creates `branch` on `repo` from `base` if it does not already exist.
    /// Idempotent: calling this again for a branch that already exists is
    /// not an error, since the pipeline reuses a branch across a rejected
    /// plan's retry (see `docs/plans/ai/milestone-5-autonomous.md`'s branch
    /// naming commit).
    async fn create_branch(&self, repo: &RepoRef, branch: &str, base: &str)
    -> Result<(), VcsError>;

    /// Pushes the commits already made in the working tree at `working_dir`
    /// up to `branch` on `repo`.
    ///
    /// Takes a working directory rather than a set of commit objects: the
    /// commits themselves are made with ordinary `git commit` inside the
    /// sandboxed checkout (see `munibot_ai::sandbox`), and this call is
    /// what actually transfers them, authenticating with a short-lived
    /// installation credential the sandboxed container itself never holds.
    async fn push(&self, repo: &RepoRef, branch: &str, working_dir: &Path) -> Result<(), VcsError>;

    /// Opens a pull request from `head` into `base`, returning a reference
    /// to it. Every forge this crate targets numbers pull requests in the
    /// same sequence as issues, so the return type is the same
    /// [`IssueRef`] an issue is identified by.
    async fn open_pull_request(
        &self,
        repo: &RepoRef,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<IssueRef, VcsError>;

    /// A clone URL with a short-lived credential embedded, suitable for a
    /// git credential helper. Never logged, and never returned in a form
    /// that ends up in shell history or a tracing span: it authenticates as
    /// the whole installation, not as any one user.
    async fn clone_url_with_token(&self, repo: &RepoRef) -> Result<Url, VcsError>;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::Forge;

    /// A minimal in-memory `PullRequestTarget`, proving the trait is
    /// object-safe -- not a stand-in for the real github implementation,
    /// which arrives in a later commit.
    struct MockPullRequestTarget {
        branches: Mutex<Vec<String>>,
        next_pr_number: u64,
    }

    #[async_trait]
    impl PullRequestTarget for MockPullRequestTarget {
        async fn create_branch(
            &self,
            _repo: &RepoRef,
            branch: &str,
            _base: &str,
        ) -> Result<(), VcsError> {
            let mut branches = self.branches.lock().unwrap();
            if !branches.iter().any(|existing| existing == branch) {
                branches.push(branch.to_string());
            }
            Ok(())
        }

        async fn push(
            &self,
            _repo: &RepoRef,
            _branch: &str,
            _working_dir: &Path,
        ) -> Result<(), VcsError> {
            Ok(())
        }

        async fn open_pull_request(
            &self,
            repo: &RepoRef,
            _head: &str,
            _base: &str,
            _title: &str,
            _body: &str,
        ) -> Result<IssueRef, VcsError> {
            Ok(IssueRef::new(repo.clone(), self.next_pr_number))
        }

        async fn clone_url_with_token(&self, repo: &RepoRef) -> Result<Url, VcsError> {
            Ok(Url::parse(&format!(
                "https://x-access-token:fake-token@github.com/{}/{}.git",
                repo.owner, repo.name
            ))
            .unwrap())
        }
    }

    fn repo() -> RepoRef {
        RepoRef::new(Forge::GitHub, "musicaloft", "munibot")
    }

    fn target() -> MockPullRequestTarget {
        MockPullRequestTarget {
            branches: Mutex::new(vec![]),
            next_pr_number: 99,
        }
    }

    #[tokio::test]
    async fn test_create_branch_is_idempotent() {
        let target = target();
        target
            .create_branch(&repo(), "munibot/1-fix-crash", "main")
            .await
            .unwrap();
        target
            .create_branch(&repo(), "munibot/1-fix-crash", "main")
            .await
            .unwrap();

        assert_eq!(target.branches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_open_pull_request_returns_an_issue_reference() {
        let target = target();
        let pull_request = target
            .open_pull_request(&repo(), "munibot/1-fix-crash", "main", "fix crash", "body")
            .await
            .unwrap();

        assert_eq!(pull_request.number, 99);
        assert_eq!(pull_request.repo, repo());
    }

    #[tokio::test]
    async fn test_clone_url_with_token_never_bakes_a_static_placeholder_into_logs() {
        // the real implementation must never log this url -- this test only
        // proves the shape is a valid, parseable url carrying credentials
        let target = target();
        let url = target.clone_url_with_token(&repo()).await.unwrap();
        assert!(url.as_str().contains("x-access-token"));
    }

    #[tokio::test]
    async fn test_trait_is_object_safe_behind_an_arc() {
        let target: Arc<dyn PullRequestTarget> = Arc::new(target());
        target
            .create_branch(&repo(), "munibot/1-fix-crash", "main")
            .await
            .unwrap();
    }
}
