//! A small-model classifier for signs of self-harm, suicidal ideation, abuse
//! disclosure, and acute distress, run on inbound messages for personas with
//! [`crate::persona::MemoryPolicy::User`] - a companion people actually
//! confide in needs this before he is public, not in a hardening pass
//! afterwards.
//!
//! [`CrisisClassifier`] only ever answers "how severe does this look", never
//! "what should happen about it" - deciding what happens on a positive
//! signal (bypassing the normal turn for a reviewed, non-generated response)
//! is a separate, later concern, kept out of this module on purpose so the
//! classifier itself stays a small, single-purpose piece that is easy to
//! test and easy to reason about in isolation.

use std::sync::Arc;

use crate::{
    provider::Provider,
    types::{AiError, CompletionRequest, History, Message, ModelParams, ModelRef},
};

/// How severe a message looks, ordered from least to most concerning.
///
/// A severity, not a boolean: "how worried should this make us" is not a
/// yes/no question, and collapsing it to one would either throw away the
/// distinction between a hard day and a stated plan, or force every
/// borderline case into the same bucket as the clearest ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CrisisSeverity {
    /// Nothing concerning: ordinary conversation, jokes, fiction that plainly
    /// is not describing something real.
    None,
    /// Real but ordinary distress - a hard day, grief, anxiety - with no
    /// indication of anything more serious.
    Low,
    /// Self-harm described without a stated plan, strongly implied (but not
    /// stated outright) suicidal ideation, an abusive situation described
    /// without naming it, or hopelessness beyond an ordinary bad day.
    Elevated,
    /// A clear statement of intent or a plan to harm oneself or someone
    /// else, an explicit disclosure of ongoing or recent abuse, or a direct
    /// expression of wanting to die.
    Severe,
}

/// The model and system prompt a [`CrisisClassifier`] uses, mirroring
/// [`crate::memory::CompactionPersona`]'s own shape and reasoning: this is a
/// single one-shot completion with no tools, budget, or handoff, not a full
/// [`crate::persona::Persona`] - and `crisis` must not depend on `persona`
/// (which depends on `harness`, which this module has no need of at all).
#[derive(Clone, Debug)]
pub struct CrisisPersona {
    pub model: ModelRef,
    pub system_prompt: String,
}

impl CrisisPersona {
    /// Builds a crisis persona using the embedded default prompt, which
    /// takes no template variables and so needs no rendering step.
    pub fn embedded(model: ModelRef) -> Self {
        Self {
            model,
            system_prompt: include_str!("../prompts/crisis_classifier.md").to_string(),
        }
    }
}

/// Classifies one inbound message's [`CrisisSeverity`].
///
/// Deliberately a **small** model: this runs on every inbound message for a
/// persona with `MemoryPolicy::User`, so it has to be cheap and fast enough
/// that adding it is not itself a reason to reconsider giving someone a
/// companion who remembers them.
pub struct CrisisClassifier {
    provider: Arc<dyn Provider>,
    persona: CrisisPersona,
}

impl CrisisClassifier {
    pub fn new(provider: Arc<dyn Provider>, persona: CrisisPersona) -> Self {
        Self { provider, persona }
    }

    /// Classifies `message`, returning [`CrisisSeverity::None`] if the
    /// classifier itself fails or answers with anything unparsable.
    ///
    /// This is a *narrower* fallback than the classifier's own tuning: it is
    /// tuned to over-trigger on genuinely ambiguous **content** - the prompt
    /// says so explicitly - but a parse failure or a provider error is
    /// evidence of a plumbing problem, not evidence about the message
    /// itself, and it has no correlation with how risky that message
    /// actually is. Defaulting a bare parsing failure to the highest
    /// severity would mean an ordinary network hiccup starts interrupting
    /// unrelated, perfectly normal conversations the moment a caller acts on
    /// this signal - a visible cost paid by everyone, for a failure mode
    /// that says nothing about any of them in particular. Every failure is
    /// still logged, so a pattern of them is visible to an operator even
    /// though no single one changes what happens to the turn it was for.
    pub async fn classify(&self, message: &str) -> CrisisSeverity {
        match self.classify_inner(message).await {
            Ok(severity) => severity,
            Err(error) => {
                tracing::warn!(%error, "crisis classifier failed; continuing without a signal");
                CrisisSeverity::None
            }
        }
    }

    async fn classify_inner(&self, message: &str) -> Result<CrisisSeverity, AiError> {
        let request = CompletionRequest::new(
            self.persona.model.clone(),
            History::from(vec![Message::user(message.to_string())]),
        )
        .with_system(self.persona.system_prompt.clone())
        // deterministic and short: the entire expected output is one word
        .with_params(ModelParams::new().with_temperature(0.0).with_max_tokens(8));

        let response = self.provider.complete(request).await?;
        Ok(parse_severity(&response.text()))
    }
}

/// Parses the classifier's one-word answer, tolerating surrounding
/// whitespace and casing since a model is never perfectly literal about
/// formatting even when explicitly told to be. Anything that isn't
/// recognized maps to [`CrisisSeverity::None`] - see
/// [`CrisisClassifier::classify`]'s own doc comment for why that is the right
/// fallback, not a compromise.
fn parse_severity(text: &str) -> CrisisSeverity {
    match text.trim().to_uppercase().as_str() {
        "SEVERE" => CrisisSeverity::Severe,
        "ELEVATED" => CrisisSeverity::Elevated,
        "LOW" => CrisisSeverity::Low,
        _ => CrisisSeverity::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    fn classifier(response_text: &str) -> CrisisClassifier {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text(response_text));
        CrisisClassifier::new(
            provider,
            CrisisPersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        )
    }

    #[test]
    fn test_severity_orders_from_none_to_severe() {
        assert!(CrisisSeverity::None < CrisisSeverity::Low);
        assert!(CrisisSeverity::Low < CrisisSeverity::Elevated);
        assert!(CrisisSeverity::Elevated < CrisisSeverity::Severe);
    }

    #[tokio::test]
    async fn test_classify_parses_each_severity_word() {
        assert_eq!(
            classifier("NONE").classify("hi").await,
            CrisisSeverity::None
        );
        assert_eq!(classifier("LOW").classify("hi").await, CrisisSeverity::Low);
        assert_eq!(
            classifier("ELEVATED").classify("hi").await,
            CrisisSeverity::Elevated
        );
        assert_eq!(
            classifier("SEVERE").classify("hi").await,
            CrisisSeverity::Severe
        );
    }

    #[tokio::test]
    async fn test_classify_tolerates_whitespace_and_casing() {
        assert_eq!(
            classifier("  severe  \n").classify("hi").await,
            CrisisSeverity::Severe
        );
        assert_eq!(
            classifier("Elevated").classify("hi").await,
            CrisisSeverity::Elevated
        );
    }

    #[tokio::test]
    async fn test_classify_falls_back_to_none_on_unparsable_output() {
        assert_eq!(
            classifier("i'm not sure, it depends").classify("hi").await,
            CrisisSeverity::None
        );
        assert_eq!(classifier("").classify("hi").await, CrisisSeverity::None);
    }

    #[tokio::test]
    async fn test_classify_falls_back_to_none_on_a_provider_error() {
        let provider: Arc<dyn Provider> =
            Arc::new(MockProvider::new().respond_error(AiError::Provider("down".to_string())));
        let classifier = CrisisClassifier::new(
            provider,
            CrisisPersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );

        assert_eq!(classifier.classify("hi").await, CrisisSeverity::None);
    }

    #[tokio::test]
    async fn test_classify_sends_the_message_as_the_user_turn() {
        let provider = Arc::new(MockProvider::new().respond_text("NONE"));
        let classifier = CrisisClassifier::new(
            provider.clone(),
            CrisisPersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );

        classifier.classify("just chatting, all good").await;

        let sent = &provider.requests()[0];
        assert_eq!(
            sent.history.iter().next().unwrap().text(),
            "just chatting, all good"
        );
        assert!(
            sent.system
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains("crisis"),
            "the embedded prompt should have been sent as the system prompt"
        );
    }

    #[tokio::test]
    async fn test_classify_uses_a_low_deterministic_temperature_and_short_output() {
        let provider = Arc::new(MockProvider::new().respond_text("NONE"));
        let classifier = CrisisClassifier::new(
            provider.clone(),
            CrisisPersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );

        classifier.classify("hi").await;

        let sent = &provider.requests()[0];
        assert_eq!(sent.params.temperature, Some(0.0));
        assert_eq!(sent.params.max_tokens, Some(8));
    }
}
