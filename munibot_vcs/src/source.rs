//! The read/comment side of a forge integration: fetching an issue and
//! talking back on its thread.

use async_trait::async_trait;

use crate::{Comment, Issue, IssueRef, VcsError};

/// Fetches issues and posts comments on them.
///
/// Object-safe (no generic methods, no `Self` return types), so the
/// pipeline holds this behind `Arc<dyn IssueSource>` and never needs to
/// know which forge it is actually talking to.
#[async_trait]
pub trait IssueSource: Send + Sync {
    /// Fetches the current state of an issue.
    async fn fetch_issue(&self, issue: &IssueRef) -> Result<Issue, VcsError>;

    /// Lists every comment on an issue, oldest first.
    async fn list_comments(&self, issue: &IssueRef) -> Result<Vec<Comment>, VcsError>;

    /// Posts a new comment on an issue.
    async fn post_comment(&self, issue: &IssueRef, body: &str) -> Result<(), VcsError>;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Utc;

    use super::*;
    use crate::{Forge, IssueState, RepoRef};

    /// A minimal in-memory `IssueSource`, proving the trait is object-safe
    /// and its methods behave as documented -- not a stand-in for the real
    /// github implementation, which arrives in a later commit.
    struct MockIssueSource {
        issue: Issue,
        comments: Mutex<Vec<Comment>>,
    }

    #[async_trait]
    impl IssueSource for MockIssueSource {
        async fn fetch_issue(&self, _issue: &IssueRef) -> Result<Issue, VcsError> {
            Ok(self.issue.clone())
        }

        async fn list_comments(&self, _issue: &IssueRef) -> Result<Vec<Comment>, VcsError> {
            Ok(self.comments.lock().unwrap().clone())
        }

        async fn post_comment(&self, _issue: &IssueRef, body: &str) -> Result<(), VcsError> {
            self.comments.lock().unwrap().push(Comment {
                author: "munibot".to_string(),
                body: body.to_string(),
                created_at: Utc::now(),
            });
            Ok(())
        }
    }

    fn issue_ref() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn source() -> MockIssueSource {
        MockIssueSource {
            issue: Issue {
                reference: issue_ref(),
                title: "bug".to_string(),
                body: "steps to reproduce".to_string(),
                author: "someone".to_string(),
                labels: vec![],
                state: IssueState::Open,
            },
            comments: Mutex::new(vec![]),
        }
    }

    #[tokio::test]
    async fn test_fetch_issue_returns_the_issue() {
        let source = source();
        let issue = source.fetch_issue(&issue_ref()).await.unwrap();
        assert_eq!(issue.title, "bug");
    }

    #[tokio::test]
    async fn test_post_comment_then_list_comments_sees_it() {
        let source = source();
        source
            .post_comment(&issue_ref(), "can you share a stack trace?")
            .await
            .unwrap();

        let comments = source.list_comments(&issue_ref()).await.unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].body, "can you share a stack trace?");
    }

    #[tokio::test]
    async fn test_trait_is_object_safe_behind_an_arc() {
        // proves IssueSource can be held as Arc<dyn IssueSource>, which is
        // exactly how the pipeline is meant to hold it
        let source: Arc<dyn IssueSource> = Arc::new(source());
        let issue = source.fetch_issue(&issue_ref()).await.unwrap();
        assert_eq!(issue.reference, issue_ref());
    }
}
