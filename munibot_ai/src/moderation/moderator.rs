use async_trait::async_trait;

use crate::types::AiError;

/// What a moderation check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModerationVerdict {
    /// Nothing flagged.
    Clear,
    /// Flagged, naming the categories the provider itself reported (e.g.
    /// `"violence"`, `"harassment"`) - never munibot's own invention, so an
    /// operator reading a safety event later sees exactly what the
    /// provider said, not a paraphrase.
    Flagged { categories: Vec<String> },
}

/// A source of content moderation verdicts.
///
/// A trait rather than [`crate::moderation::OpenAiModerator`] used
/// directly, so a unit test can substitute a scripted fake - the same
/// reasoning [`crate::provider::Provider`] and
/// [`crate::tools::exa::ExaBackend`] already exist as traits for.
///
/// An `Err` here means the check itself failed to run (a network error, an
/// outage, an auth failure) - never used to represent flagged content,
/// which is [`ModerationVerdict::Flagged`] instead.
/// [`crate::moderation::ModerationGate`] is what decides what an `Err` here
/// means for a turn.
#[async_trait]
pub trait Moderator: Send + Sync {
    async fn moderate(&self, text: &str) -> Result<ModerationVerdict, AiError>;
}
