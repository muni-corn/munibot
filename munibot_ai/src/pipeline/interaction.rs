//! `InteractionAdapter`: how a paused pipeline actually reaches a human,
//! and how their reply resumes it.
//!
//! A `RequestPlanHelp` or `RequestBuildHelp` handoff already moves a run
//! to `AwaitingUserInput` and persists that (see the executor and advance
//! commits) -- the executor itself simply stops looping the moment it
//! sees that state, spending nothing further while paused. This module is
//! what actually delivers the question somewhere a person can answer it,
//! and turns their answer back into the `UserInputReceived` event that
//! resumes the run.

use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;
use thiserror::Error;

use crate::pipeline::{InteractionRequest, PipelineId};

/// Why delivering a question, or a maintainer's answer to it, failed.
#[derive(Error, Debug)]
pub enum InteractionError {
    #[error("couldn't deliver a question for {0:?}: {1}")]
    Delivery(PipelineId, String),
    #[error("couldn't send a notification about {0:?}: {1}")]
    Notification(PipelineId, String),
}

/// A human's answer to one [`InteractionRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractionResponse {
    pub response: String,
}

impl InteractionResponse {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

/// Delivers a paused pipeline's question to a human, and delivers
/// informational messages about a run that expect no reply.
///
/// `request_input` returns only once a human has actually answered --
/// never by polling or busy-waiting. How a real adapter suspends until
/// that happens (a signed-in maintainer's own chat reply resolving a
/// pending future, a github webhook firing once a comment lands) is
/// entirely its own concern; the executor that calls this trait never
/// needs to know.
#[async_trait]
pub trait InteractionAdapter: Send + Sync {
    async fn request_input(
        &self,
        pipeline_id: PipelineId,
        request: &InteractionRequest,
    ) -> Result<InteractionResponse, InteractionError>;

    /// Sends an informational message about `pipeline_id` -- a final
    /// result, a warning -- expecting no reply, unlike `request_input`.
    async fn notify(&self, pipeline_id: PipelineId, message: &str) -> Result<(), InteractionError>;
}

/// A scripted [`InteractionAdapter`], for tests -- answers are queued in
/// advance rather than actually waiting on anything, and every call is
/// recorded so a test can assert on what was actually asked or said, the
/// same reasoning [`crate::pipeline::MockAgentDispatcher`] already
/// applies one layer up.
#[derive(Default)]
pub struct MockInteractionAdapter {
    answers: Mutex<VecDeque<Result<InteractionResponse, InteractionError>>>,
    requests: Mutex<Vec<(PipelineId, InteractionRequest)>>,
    notifications: Mutex<Vec<(PipelineId, String)>>,
}

impl MockInteractionAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn answer(self, response: Result<InteractionResponse, InteractionError>) -> Self {
        self.answers
            .lock()
            .expect("mock lock poisoned")
            .push_back(response);
        self
    }

    pub fn requests(&self) -> Vec<(PipelineId, InteractionRequest)> {
        self.requests.lock().expect("mock lock poisoned").clone()
    }

    pub fn notifications(&self) -> Vec<(PipelineId, String)> {
        self.notifications
            .lock()
            .expect("mock lock poisoned")
            .clone()
    }
}

#[async_trait]
impl InteractionAdapter for MockInteractionAdapter {
    async fn request_input(
        &self,
        pipeline_id: PipelineId,
        request: &InteractionRequest,
    ) -> Result<InteractionResponse, InteractionError> {
        self.requests
            .lock()
            .expect("mock lock poisoned")
            .push((pipeline_id, request.clone()));

        self.answers
            .lock()
            .expect("mock lock poisoned")
            .pop_front()
            .unwrap_or_else(|| panic!("MockInteractionAdapter ran out of scripted answers"))
    }

    async fn notify(&self, pipeline_id: PipelineId, message: &str) -> Result<(), InteractionError> {
        self.notifications
            .lock()
            .expect("mock lock poisoned")
            .push((pipeline_id, message.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> InteractionRequest {
        InteractionRequest {
            prompt: "which database should this use?".to_string(),
        }
    }

    #[tokio::test]
    async fn test_mock_adapter_returns_scripted_answers_in_order() {
        let adapter = MockInteractionAdapter::new()
            .answer(Ok(InteractionResponse::new("postgres")))
            .answer(Ok(InteractionResponse::new("redis")));

        let first = adapter
            .request_input(PipelineId(1), &request())
            .await
            .unwrap();
        assert_eq!(first.response, "postgres");

        let second = adapter
            .request_input(PipelineId(1), &request())
            .await
            .unwrap();
        assert_eq!(second.response, "redis");
    }

    #[tokio::test]
    async fn test_mock_adapter_records_every_request() {
        let adapter = MockInteractionAdapter::new().answer(Ok(InteractionResponse::new("yes")));
        adapter
            .request_input(PipelineId(7), &request())
            .await
            .unwrap();

        let requests = adapter.requests();
        assert_eq!(requests, vec![(PipelineId(7), request())]);
    }

    #[tokio::test]
    async fn test_mock_adapter_records_every_notification() {
        let adapter = MockInteractionAdapter::new();
        adapter
            .notify(PipelineId(7), "opened a pull request")
            .await
            .unwrap();

        assert_eq!(adapter.notifications(), vec![(
            PipelineId(7),
            "opened a pull request".to_string()
        )]);
    }

    #[tokio::test]
    #[should_panic(expected = "ran out of scripted answers")]
    async fn test_mock_adapter_panics_when_it_runs_out_of_scripted_answers() {
        let adapter = MockInteractionAdapter::new();
        adapter.request_input(PipelineId(1), &request()).await.ok();
    }

    #[test]
    fn test_interaction_response_new_wraps_the_text() {
        assert_eq!(InteractionResponse::new("yes").response, "yes");
    }
}
