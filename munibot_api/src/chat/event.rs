use serde::{Deserialize, Serialize};

/// One step of progress through a reply, streamed to the chat page over SSE.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ChatEvent {
    /// A chunk of the reply's text.
    TextDelta(String),
    /// The reply is complete. Without this, a dropped connection would look
    /// exactly like a reply that simply ended.
    Finished,
    /// The reply failed partway through. The text is the error's own
    /// friendly, lowercase message, safe to show as is.
    Failed(String),
}

#[cfg(feature = "server")]
impl ChatEvent {
    /// Turns a reply's chunks into the events the chat page consumes: each
    /// chunk becomes a [`Self::TextDelta`], then the stream ends with
    /// [`Self::Finished`], or with [`Self::Failed`] at the first error,
    /// whichever comes first.
    pub fn stream_from_reply(
        reply: futures::stream::BoxStream<'static, Result<String, munibot_ai::AiError>>,
    ) -> futures::stream::BoxStream<'static, Self> {
        use futures::StreamExt;

        // `None` once the terminal event has gone out, so nothing is ever
        // emitted after `Finished` or `Failed`
        futures::stream::unfold(Some(reply), |state| async move {
            let mut reply = state?;
            match reply.next().await {
                Some(Ok(text)) => Some((Self::TextDelta(text), Some(reply))),
                Some(Err(error)) => Some((Self::Failed(error.to_string()), None)),
                None => Some((Self::Finished, None)),
            }
        })
        .boxed()
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use futures::{StreamExt, stream};
    use munibot_ai::AiError;

    use super::*;

    async fn events(chunks: Vec<Result<String, AiError>>) -> Vec<ChatEvent> {
        ChatEvent::stream_from_reply(stream::iter(chunks).boxed())
            .collect()
            .await
    }

    #[tokio::test]
    async fn test_text_chunks_become_deltas_then_finished() {
        let events = events(vec![Ok("hi ".to_string()), Ok("there".to_string())]).await;

        assert_eq!(events, vec![
            ChatEvent::TextDelta("hi ".to_string()),
            ChatEvent::TextDelta("there".to_string()),
            ChatEvent::Finished,
        ]);
    }

    #[test]
    fn test_tool_finished_converts_duration_to_milliseconds() {
        let event: ChatEvent = HarnessEvent::ToolFinished {
            name: "web_search".to_string(),
            duration: Duration::from_millis(250),
            ok: true,
            result: "three results found".to_string(),
        }
        .into();
        assert_eq!(event, ChatEvent::ToolFinished {
            name: "web_search".to_string(),
            duration_ms: 250,
            ok: true,
            result: "three results found".to_string(),
        });
    }

    #[test]
    fn test_iteration_complete_collapses_usage_into_a_total() {
        let event: ChatEvent = HarnessEvent::IterationComplete {
            iteration: 2,
            usage: munibot_ai::types::Usage::new(10, 20),
        }
        .into();
        assert_eq!(event, ChatEvent::IterationComplete {
            iteration: 2,
            usage: ChatUsage {
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
            },
        });
    }

    #[test]
    fn test_handoff_carries_the_payload_verbatim() {
        let event: ChatEvent =
            HarnessEvent::Handoff(serde_json::json!({"action": "ApprovePlan"})).into();
        assert_eq!(
            event,
            ChatEvent::Handoff(serde_json::json!({"action": "ApprovePlan"}))
        );
    }

    #[test]
    fn test_turn_finished_maps_usage_and_cost() {
        let event: ChatEvent = HarnessEvent::TurnFinished {
            usage: munibot_ai::types::Usage::new(5, 5),
            cost: munibot_ai::types::Cost::from_micros(1200),
        }
        .into();
        assert_eq!(event, ChatEvent::TurnFinished {
            usage: ChatUsage {
                input_tokens: 5,
                output_tokens: 5,
                total_tokens: 10,
            },
            cost_micros: 1200,
        });
    }

    #[test]
    fn test_failed_carries_the_error_s_friendly_message_not_the_error_type() {
        let event: ChatEvent = HarnessEvent::Failed(AiError::Cancelled).into();
        assert_eq!(event, ChatEvent::Failed {
            message: AiError::Cancelled.to_string(),
            kind: ChatFailureKind::Other,
        });
    }

    #[test]
    fn test_budget_exceeded_maps_to_its_own_kind() {
        let error = AiError::BudgetExceeded {
            limit: "8 iterations".to_string(),
        };
        assert_eq!(
            ChatFailureKind::from(&error),
            ChatFailureKind::BudgetExceeded
        );
    }

    #[test]
    fn test_a_transient_provider_error_maps_to_its_own_kind() {
        assert_eq!(
            ChatFailureKind::from(&AiError::Provider("gateway timeout".to_string())),
            ChatFailureKind::Transient
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::RateLimited { retry_after: None }),
            ChatFailureKind::Transient
        );
    }

    #[test]
    fn test_everything_else_falls_back_to_other() {
        assert_eq!(
            ChatFailureKind::from(&AiError::Cancelled),
            ChatFailureKind::Other
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::Rejected("bad key".to_string())),
            ChatFailureKind::Other
        );
        assert_eq!(
            ChatFailureKind::from(&AiError::Tool("boom".to_string())),
            ChatFailureKind::Other
        );
    }
}
