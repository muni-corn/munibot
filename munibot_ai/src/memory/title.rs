use std::sync::Arc;

use crate::{
    provider::Provider,
    types::{AiError, CompletionRequest, History, Message, ModelParams, ModelRef},
};

/// The model and system prompt a [`TitleGenerator`] uses, mirroring
/// [`crate::memory::CompactionPersona`]'s own shape and reasoning: a single
/// one-shot completion with no tools, budget, or handoff, not a full
/// [`crate::persona::Persona`].
#[derive(Clone, Debug)]
pub struct TitlePersona {
    pub model: ModelRef,
    pub system_prompt: String,
}

impl TitlePersona {
    /// Builds a title persona using the embedded default prompt, which
    /// takes no template variables and so needs no rendering step.
    pub fn embedded(model: ModelRef) -> Self {
        Self {
            model,
            system_prompt: include_str!("../../prompts/title.md").to_string(),
        }
    }
}

/// Names a conversation from its first exchange with a cheap, hard-capped
/// single-iteration call.
///
/// Deliberately cloneable (both fields are): [`crate::service::Ai`] runs
/// generation in a detached background task rather than delaying a turn's
/// own response on it, and a spawned task needs its own owned copy rather
/// than a borrow of `&self`.
#[derive(Clone)]
pub struct TitleGenerator {
    provider: Arc<dyn Provider>,
    persona: TitlePersona,
}

impl TitleGenerator {
    pub fn new(provider: Arc<dyn Provider>, persona: TitlePersona) -> Self {
        Self { provider, persona }
    }

    /// Generates a title from a conversation's first exchange.
    pub async fn generate(
        &self,
        user_message: &str,
        assistant_reply: &str,
    ) -> Result<String, AiError> {
        let exchange = format!("User: {user_message}\n\nAssistant: {assistant_reply}");

        let request = CompletionRequest::new(
            self.persona.model.clone(),
            History::from(vec![Message::user(exchange)]),
        )
        .with_system(self.persona.system_prompt.clone())
        // a title is a handful of words - this is cheap and hard-capped on
        // purpose, not a real conversation the model gets to run long
        .with_params(ModelParams::new().with_temperature(0.3).with_max_tokens(24));

        let response = self.provider.complete(request).await?;
        let title = response.text().trim().trim_matches('"').to_string();

        if title.is_empty() {
            return Err(AiError::Other(
                "the title generation model returned an empty title :<".to_string(),
            ));
        }

        Ok(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    fn generator(response_text: &str) -> (TitleGenerator, Arc<MockProvider>) {
        let provider = Arc::new(MockProvider::new().respond_text(response_text));
        let generator = TitleGenerator::new(
            provider.clone(),
            TitlePersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );
        (generator, provider)
    }

    #[tokio::test]
    async fn test_generate_returns_the_model_s_title() {
        let (generator, _) = generator("weekend hiking plans");
        let title = generator
            .generate(
                "want to go hiking this weekend?",
                "sure, where were you thinking?",
            )
            .await
            .expect("should succeed");
        assert_eq!(title, "weekend hiking plans");
    }

    #[tokio::test]
    async fn test_generate_trims_surrounding_whitespace_and_quotes() {
        let (generator, _) = generator("  \"weekend hiking plans\"\n");
        let title = generator
            .generate("hi", "hi there")
            .await
            .expect("should succeed");
        assert_eq!(title, "weekend hiking plans");
    }

    #[tokio::test]
    async fn test_generate_fails_on_an_empty_title() {
        let (generator, _) = generator("   ");
        let result = generator.generate("hi", "hi there").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generate_propagates_a_provider_error() {
        let provider: Arc<dyn Provider> =
            Arc::new(MockProvider::new().respond_error(AiError::Provider("down".to_string())));
        let generator = TitleGenerator::new(
            provider,
            TitlePersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );
        let result = generator.generate("hi", "hi there").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generate_sends_both_sides_of_the_exchange() {
        let (generator, provider) = generator("weekend hiking plans");
        generator
            .generate("want to go hiking?", "sure, when works for you?")
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        let text = sent.history.iter().next().unwrap().text();
        assert!(text.contains("want to go hiking?"));
        assert!(text.contains("sure, when works for you?"));
    }

    #[tokio::test]
    async fn test_generate_uses_a_small_hard_capped_request() {
        let (generator, provider) = generator("weekend hiking plans");
        generator
            .generate("hi", "hi there")
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        assert_eq!(sent.params.max_tokens, Some(24));
    }
}
