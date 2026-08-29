//! `GitHubCommentAdapter`: the fallback [`InteractionAdapter`] for when
//! there is no signed-in maintainer to ask in chat -- posts the question
//! as an issue comment and resumes once a maintainer replies on the same
//! thread.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use munibot_vcs::{IssueRef, IssueSource};
use tokio::time::sleep;

use crate::pipeline::{
    InteractionAdapter, InteractionError, InteractionRequest, InteractionResponse, PipelineId,
};

/// How often [`GitHubCommentAdapter::request_input`] checks for a reply,
/// by default. Github's own rate limits make anything much shorter than
/// this wasteful for what is, in practice, a wait measured in minutes to
/// hours rather than seconds.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Delivers a pipeline's question as a comment on the issue that
/// triggered it, and resumes once anyone other than munibot itself
/// replies on that same issue.
///
/// "Matching on the comment thread" means exactly this: any comment
/// posted after the question was, by someone who isn't the bot -- not a
/// reply to a specific comment id, which not every forge's own comment
/// model even has. This is the fallback path for when nobody is signed in
/// to answer in the web chat (see the web chat interaction adapter); a
/// public repository's issue thread is the one place a question is
/// guaranteed to reach whoever is actually watching it.
pub struct GitHubCommentAdapter {
    issue_source: Arc<dyn IssueSource>,
    issue: IssueRef,
    bot_login: String,
    poll_interval: Duration,
}

impl GitHubCommentAdapter {
    pub fn new(
        issue_source: Arc<dyn IssueSource>,
        issue: IssueRef,
        bot_login: impl Into<String>,
    ) -> Self {
        Self {
            issue_source,
            issue,
            bot_login: bot_login.into(),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Overrides the default poll interval -- tests use this to avoid an
    /// actual multi-second wait.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }
}

#[async_trait]
impl InteractionAdapter for GitHubCommentAdapter {
    async fn request_input(
        &self,
        pipeline_id: PipelineId,
        request: &InteractionRequest,
    ) -> Result<InteractionResponse, InteractionError> {
        self.issue_source
            .post_comment(&self.issue, &request.prompt)
            .await
            .map_err(|error| InteractionError::Delivery(pipeline_id, error.to_string()))?;

        let already_seen = self
            .issue_source
            .list_comments(&self.issue)
            .await
            .map_err(|error| InteractionError::Delivery(pipeline_id, error.to_string()))?
            .len();

        loop {
            let comments = self
                .issue_source
                .list_comments(&self.issue)
                .await
                .map_err(|error| InteractionError::Delivery(pipeline_id, error.to_string()))?;

            let reply = comments
                .iter()
                .skip(already_seen)
                .find(|comment| !comment.author.eq_ignore_ascii_case(&self.bot_login));

            if let Some(reply) = reply {
                return Ok(InteractionResponse::new(reply.body.clone()));
            }

            sleep(self.poll_interval).await;
        }
    }

    async fn notify(&self, pipeline_id: PipelineId, message: &str) -> Result<(), InteractionError> {
        self.issue_source
            .post_comment(&self.issue, message)
            .await
            .map_err(|error| InteractionError::Notification(pipeline_id, error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Utc;
    use munibot_vcs::{Comment, Forge, Issue, IssueState, RepoRef, VcsError};

    use super::*;

    /// A minimal fake `IssueSource`, entirely local to this test module --
    /// `munibot_vcs`'s own mock is private to its crate's tests, and this
    /// adapter's tests need one that lets a test append a "reply" mid-poll.
    struct FakeIssueSource {
        comments: Mutex<Vec<Comment>>,
    }

    impl FakeIssueSource {
        fn new() -> Self {
            Self {
                comments: Mutex::new(vec![]),
            }
        }

        fn push_reply(&self, author: &str, body: &str) {
            self.comments.lock().unwrap().push(Comment {
                author: author.to_string(),
                body: body.to_string(),
                created_at: Utc::now(),
            });
        }
    }

    #[async_trait]
    impl IssueSource for FakeIssueSource {
        async fn fetch_issue(&self, issue: &IssueRef) -> Result<Issue, VcsError> {
            Ok(Issue {
                reference: issue.clone(),
                title: "an issue".to_string(),
                body: String::new(),
                author: "someone".to_string(),
                labels: vec![],
                state: IssueState::Open,
            })
        }

        async fn list_comments(&self, _issue: &IssueRef) -> Result<Vec<Comment>, VcsError> {
            Ok(self.comments.lock().unwrap().clone())
        }

        async fn post_comment(&self, _issue: &IssueRef, body: &str) -> Result<(), VcsError> {
            self.push_reply("munibot[bot]", body);
            Ok(())
        }
    }

    fn issue() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn request() -> InteractionRequest {
        InteractionRequest {
            prompt: "which database should this use?".to_string(),
        }
    }

    #[tokio::test]
    async fn test_request_input_posts_the_question_as_a_comment() {
        let source = Arc::new(FakeIssueSource::new());
        let adapter = GitHubCommentAdapter::new(source.clone(), issue(), "munibot[bot]")
            .with_poll_interval(Duration::from_millis(1));

        let source_for_task = source.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            source_for_task.push_reply("a-maintainer", "postgres");
        });

        adapter
            .request_input(PipelineId(1), &request())
            .await
            .unwrap();
        handle.await.unwrap();

        let comments = source.comments.lock().unwrap();
        assert!(
            comments
                .iter()
                .any(|comment| comment.body == request().prompt),
            "the question itself should have been posted as a comment"
        );
    }

    #[tokio::test]
    async fn test_request_input_ignores_the_bots_own_comment_and_waits_for_a_real_reply() {
        let source = Arc::new(FakeIssueSource::new());
        let adapter = GitHubCommentAdapter::new(source.clone(), issue(), "munibot[bot]")
            .with_poll_interval(Duration::from_millis(1));

        let source_for_task = source.clone();
        let handle = tokio::spawn(async move {
            // give the adapter a moment to post its own question and start
            // polling before the "reply" shows up
            tokio::time::sleep(Duration::from_millis(20)).await;
            source_for_task.push_reply("a-maintainer", "use redis");
        });

        let response = adapter
            .request_input(PipelineId(1), &request())
            .await
            .unwrap();
        handle.await.unwrap();
        assert_eq!(response.response, "use redis");
    }

    #[tokio::test]
    async fn test_request_input_ignores_replies_that_predate_the_question() {
        // an unrelated, earlier comment from a human should never be
        // mistaken for an answer to a question not yet asked
        let source = Arc::new(FakeIssueSource::new());
        source.push_reply("a-maintainer", "totally unrelated earlier comment");

        let adapter = GitHubCommentAdapter::new(source.clone(), issue(), "munibot[bot]")
            .with_poll_interval(Duration::from_millis(1));

        let source_for_task = source.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            source_for_task.push_reply("a-maintainer", "the real answer");
        });

        let response = adapter
            .request_input(PipelineId(1), &request())
            .await
            .unwrap();
        handle.await.unwrap();
        assert_eq!(response.response, "the real answer");
    }

    #[tokio::test]
    async fn test_notify_posts_a_comment_with_no_reply_expected() {
        let source = Arc::new(FakeIssueSource::new());
        let adapter = GitHubCommentAdapter::new(source.clone(), issue(), "munibot[bot]");

        adapter
            .notify(PipelineId(1), "opened a pull request")
            .await
            .unwrap();

        let comments = source.comments.lock().unwrap();
        assert!(
            comments
                .iter()
                .any(|comment| comment.body == "opened a pull request")
        );
    }
}
