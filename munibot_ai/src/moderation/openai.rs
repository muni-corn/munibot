use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    moderation::{ModerationVerdict, Moderator},
    types::AiError,
};

const OPENAI_MODERATIONS_URL: &str = "https://api.openai.com/v1/moderations";

/// Same reasoning as `crate::tools::exa::ExaClient`'s own timeout: a
/// moderation check sits inline before a turn even starts (or before its
/// reply is stored), so a hung request must not be allowed to hang the
/// whole turn indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// User-Agent sent with every request, matching every other outbound HTTP
/// client in this crate (see `crate::tools::exa::ExaClient`).
const USER_AGENT: &str = concat!(
    "munibot/",
    env!("CARGO_PKG_VERSION"),
    " (https://git.musicaloft.com/municorn/munibot)",
);

/// The moderation model to request. `omni-moderation-latest` is OpenAI's
/// current multi-category model as of this writing; pinned to `-latest`
/// rather than a dated snapshot since a moderation model improving out
/// from under this integration is a feature, not a risk, the way it would
/// be for a completion model whose exact behaviour a persona depends on.
const MODERATION_MODEL: &str = "omni-moderation-latest";

#[derive(Serialize)]
struct ModerationRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct ModerationResponse {
    results: Vec<ModerationResult>,
}

#[derive(Deserialize)]
struct ModerationResult {
    flagged: bool,
    /// A map of category name to whether it was flagged - `HashMap` rather
    /// than a fixed struct, since OpenAI has added categories before and a
    /// new one appearing must never fail deserialization.
    categories: std::collections::HashMap<String, bool>,
}

/// A [`Moderator`] backed by OpenAI's moderation endpoint.
///
/// The one provider integration this crate ships out of the box: OpenAI is,
/// as of this writing, the only provider munibot supports with a dedicated
/// moderation endpoint at all (see `crate::moderation`'s own doc comment).
/// Reuses `OPENAI_API_KEY` - the same credential `ProviderResolver` already
/// requires for an `openai:`-prefixed model - rather than a second,
/// moderation-specific key, since it is the same OpenAI account either way.
#[derive(Clone)]
pub struct OpenAiModerator {
    client: Client,
    api_key: String,
}

impl std::fmt::Debug for OpenAiModerator {
    /// Redacts the API key, the same reasoning `ExaClient`'s own `Debug`
    /// impl documents.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiModerator")
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl OpenAiModerator {
    /// Builds a moderator from an explicit API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            // reqwest only fails to build a Client when TLS initialization fails, which
            // would be a fatal startup error anyway - the same reasoning ExaClient::new
            // documents for its own identical expect()
            .expect("failed to build reqwest client for openai moderation");

        Self {
            client,
            api_key: api_key.into(),
        }
    }

    /// Builds a moderator from the `OPENAI_API_KEY` environment variable,
    /// or `None` if it isn't set - moderation is opt-in infrastructure, not
    /// something that should fail startup over.
    pub fn from_env() -> Option<Self> {
        std::env::var("OPENAI_API_KEY").ok().map(Self::new)
    }
}

#[async_trait]
impl Moderator for OpenAiModerator {
    async fn moderate(&self, text: &str) -> Result<ModerationVerdict, AiError> {
        let response = self
            .client
            .post(OPENAI_MODERATIONS_URL)
            .bearer_auth(&self.api_key)
            .json(&ModerationRequest {
                model: MODERATION_MODEL,
                input: text,
            })
            .send()
            .await
            .map_err(|error| AiError::Provider(format!("moderation request failed :< {error}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(AiError::Provider(format!(
                "openai moderation rejected the request (http {status})"
            )));
        }

        let body: ModerationResponse = response.json().await.map_err(|error| {
            AiError::Provider(format!(
                "couldn't parse openai's moderation response :< {error}"
            ))
        })?;

        let Some(result) = body.results.into_iter().next() else {
            return Err(AiError::Provider(
                "openai's moderation response had no results".to_string(),
            ));
        };

        if !result.flagged {
            return Ok(ModerationVerdict::Clear);
        }

        let mut categories: Vec<String> = result
            .categories
            .into_iter()
            .filter_map(|(category, flagged)| flagged.then_some(category))
            .collect();
        categories.sort();

        Ok(ModerationVerdict::Flagged { categories })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_the_api_key() {
        let moderator = OpenAiModerator::new("sk-super-secret");
        assert!(!format!("{moderator:?}").contains("sk-super-secret"));
    }

    #[test]
    fn test_from_env_is_none_without_a_key() {
        // SAFETY: test-only, and no other test in this process reads or
        // writes OPENAI_API_KEY concurrently with this one
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        assert!(OpenAiModerator::from_env().is_none());
    }
}
