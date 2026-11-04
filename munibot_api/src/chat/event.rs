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

    #[tokio::test]
    async fn test_an_empty_reply_still_finishes() {
        assert_eq!(events(vec![]).await, vec![ChatEvent::Finished]);
    }

    #[tokio::test]
    async fn test_an_error_ends_the_stream_with_failed_and_nothing_after() {
        let events = events(vec![
            Ok("par".to_string()),
            Err(AiError::Rejected("bad api key".to_string())),
            Ok("never seen".to_string()),
        ])
        .await;

        assert_eq!(events.len(), 2, "nothing should follow a failure");
        assert_eq!(events[0], ChatEvent::TextDelta("par".to_string()));
        assert!(
            matches!(&events[1], ChatEvent::Failed(message) if message.contains("bad api key"))
        );
    }
}
