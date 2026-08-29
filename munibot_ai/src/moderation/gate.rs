use std::sync::Arc;

use crate::{
    moderation::{ModerationPolicy, ModerationVerdict, Moderator},
    types::AiError,
};

/// Runs a moderation check and turns its outcome into a turn-level result,
/// applying [`ModerationPolicy`] to a check failure - the actual
/// logic-with-consequences layer over the bare [`Moderator`] trait, so
/// [`crate::service::Ai`] never has to inline the fail-open/fail-closed
/// branching itself, both for the pre-check and the post-check.
pub struct ModerationGate {
    moderator: Arc<dyn Moderator>,
}

impl ModerationGate {
    pub fn new(moderator: Arc<dyn Moderator>) -> Self {
        Self { moderator }
    }

    /// Checks `text` against `policy`.
    ///
    /// Flagged content always refuses, regardless of `policy` - that
    /// distinction only ever governs what happens when the check *itself*
    /// fails to run (see [`ModerationPolicy`]'s own doc comment), never
    /// what happens once it actually flags something.
    pub async fn check(&self, policy: ModerationPolicy, text: &str) -> Result<(), AiError> {
        match self.moderator.moderate(text).await {
            Ok(ModerationVerdict::Clear) => Ok(()),
            Ok(ModerationVerdict::Flagged { categories }) => {
                tracing::warn!(
                    ?categories,
                    "provider moderation flagged content; refusing the turn"
                );
                Err(AiError::Refused(format!(
                    "that got flagged by moderation ({})",
                    categories.join(", ")
                )))
            }
            Err(error) => match policy {
                ModerationPolicy::FailOpen => {
                    tracing::warn!(%error, "moderation check failed; allowing the turn anyway");
                    Ok(())
                }
                ModerationPolicy::FailClosed => {
                    tracing::warn!(%error, "moderation check failed; refusing the turn");
                    Err(AiError::Refused(
                        "moderation is unavailable right now, and this persona refuses rather \
                         than let something through unchecked"
                            .to_string(),
                    ))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    struct ScriptedModerator(Result<ModerationVerdict, AiError>);

    #[async_trait]
    impl Moderator for ScriptedModerator {
        async fn moderate(&self, _text: &str) -> Result<ModerationVerdict, AiError> {
            match &self.0 {
                Ok(verdict) => Ok(verdict.clone()),
                Err(_) => Err(AiError::Provider("moderation endpoint is down".to_string())),
            }
        }
    }

    fn gate(result: Result<ModerationVerdict, AiError>) -> ModerationGate {
        ModerationGate::new(Arc::new(ScriptedModerator(result)))
    }

    #[tokio::test]
    async fn test_clear_content_is_never_refused_under_either_policy() {
        let gate = gate(Ok(ModerationVerdict::Clear));
        gate.check(ModerationPolicy::FailOpen, "hello")
            .await
            .expect("clear content should pass under fail-open");
        gate.check(ModerationPolicy::FailClosed, "hello")
            .await
            .expect("clear content should pass under fail-closed");
    }

    #[tokio::test]
    async fn test_flagged_content_is_always_refused_regardless_of_policy() {
        let gate = gate(Ok(ModerationVerdict::Flagged {
            categories: vec!["violence".to_string()],
        }));
        assert!(gate.check(ModerationPolicy::FailOpen, "hi").await.is_err());
        assert!(
            gate.check(ModerationPolicy::FailClosed, "hi")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_a_check_failure_is_allowed_under_fail_open() {
        let gate = gate(Err(AiError::Provider("down".to_string())));
        gate.check(ModerationPolicy::FailOpen, "hi")
            .await
            .expect("a check failure should fail open");
    }

    #[tokio::test]
    async fn test_a_check_failure_is_refused_under_fail_closed() {
        let gate = gate(Err(AiError::Provider("down".to_string())));
        assert!(
            gate.check(ModerationPolicy::FailClosed, "hi")
                .await
                .is_err(),
            "a check failure should refuse the turn under fail-closed"
        );
    }
}
