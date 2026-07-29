use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a model reference could not be parsed.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ModelRefError {
    #[error(
        "model reference {0:?} needs a `provider:model` shape, like `anthropic:claude-opus-5` :<"
    )]
    MissingSeparator(String),
    #[error("model reference {0:?} is missing a provider before the colon :<")]
    EmptyProvider(String),
    #[error("model reference {0:?} is missing a model after the colon :<")]
    EmptyModel(String),
}

/// Which provider and model to use.
///
/// Written as `provider:model` everywhere a human touches it, which is what
/// lets a persona pick its provider with a single string in configuration.
///
/// # Example
/// ```
/// use munibot_ai_types::ModelRef;
///
/// let model: ModelRef = "anthropic:claude-opus-5".parse().unwrap();
/// assert_eq!(model.provider(), "anthropic");
/// assert_eq!(model.model(), "claude-opus-5");
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct ModelRef {
    provider: String,
    model: String,
}

impl ModelRef {
    /// Builds a reference from an already-split provider and model.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }

    /// The provider half, such as `anthropic`.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The model half, such as `claude-opus-5`.
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl FromStr for ModelRef {
    type Err = ModelRefError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (provider, model) = raw
            .split_once(':')
            .ok_or_else(|| ModelRefError::MissingSeparator(raw.to_string()))?;

        // a half-written reference is a configuration mistake worth naming precisely,
        // because it otherwise surfaces much later as a confusing "unknown
        // provider"
        if provider.trim().is_empty() {
            return Err(ModelRefError::EmptyProvider(raw.to_string()));
        }
        if model.trim().is_empty() {
            return Err(ModelRefError::EmptyModel(raw.to_string()));
        }

        Ok(Self::new(provider.trim(), model.trim()))
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.provider, self.model)
    }
}

impl TryFrom<String> for ModelRef {
    type Error = ModelRefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ModelRef> for String {
    fn from(value: ModelRef) -> Self {
        value.to_string()
    }
}

/// Sampling and length knobs for one request.
///
/// Every field is optional so that a persona only states what it cares about
/// and the provider's own default covers the rest.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ModelParams {
    /// Sampling temperature. Higher is more varied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling cutoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Maximum tokens to generate in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Token budget for extended reasoning, on providers that expose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,
}

impl ModelParams {
    /// Builds params with no overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the sampling temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the response length ceiling.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Sets the extended reasoning budget.
    pub fn with_thinking_budget(mut self, thinking_budget: u32) -> Self {
        self.thinking_budget = Some(thinking_budget);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_a_well_formed_reference() {
        let model: ModelRef = "anthropic:claude-opus-5".parse().expect("should parse");
        assert_eq!(
            model.provider(),
            "anthropic",
            "provider should be the first half"
        );
        assert_eq!(
            model.model(),
            "claude-opus-5",
            "model should be the second half"
        );
    }

    #[test]
    fn test_display_roundtrips() {
        let raw = "openai:gpt-5.2";
        let model: ModelRef = raw.parse().expect("should parse");
        assert_eq!(model.to_string(), raw, "display should reproduce the input");
    }

    #[test]
    fn test_model_half_may_contain_colons() {
        // ollama and some gateways use tags like `llama3:70b-instruct`
        let model: ModelRef = "ollama:llama3:70b".parse().expect("should parse");
        assert_eq!(model.provider(), "ollama", "only the first colon separates");
        assert_eq!(model.model(), "llama3:70b", "the rest belongs to the model");
    }

    #[test]
    fn test_surrounding_whitespace_is_trimmed() {
        let model: ModelRef = " anthropic : claude-opus-5 ".parse().expect("should parse");
        assert_eq!(
            model.to_string(),
            "anthropic:claude-opus-5",
            "whitespace from hand-edited config should not survive"
        );
    }

    #[test]
    fn test_rejects_a_reference_without_a_separator() {
        let error = "claude-opus-5"
            .parse::<ModelRef>()
            .expect_err("should reject");
        assert!(
            matches!(error, ModelRefError::MissingSeparator(_)),
            "expected a missing separator error, got {error:?}"
        );
    }

    #[test]
    fn test_rejects_an_empty_provider() {
        let error = ":claude-opus-5"
            .parse::<ModelRef>()
            .expect_err("should reject");
        assert!(
            matches!(error, ModelRefError::EmptyProvider(_)),
            "expected an empty provider error, got {error:?}"
        );
    }

    #[test]
    fn test_rejects_an_empty_model() {
        let error = "anthropic:".parse::<ModelRef>().expect_err("should reject");
        assert!(
            matches!(error, ModelRefError::EmptyModel(_)),
            "expected an empty model error, got {error:?}"
        );
    }

    #[test]
    fn test_serializes_as_a_plain_string() {
        // this is what lets config say `model = "anthropic:claude-opus-5"`
        let model = ModelRef::new("anthropic", "claude-opus-5");
        let encoded = serde_json::to_value(&model).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!("anthropic:claude-opus-5"),
            "a model reference should be a string on the wire, not an object"
        );
    }

    #[test]
    fn test_deserializes_from_a_plain_string() {
        let model: ModelRef = serde_json::from_value(serde_json::json!("openai:gpt-5.2"))
            .expect("should deserialize");
        assert_eq!(
            model,
            ModelRef::new("openai", "gpt-5.2"),
            "roundtrip should hold"
        );
    }

    #[test]
    fn test_deserializing_a_bad_reference_fails_loudly() {
        let result: Result<ModelRef, _> = serde_json::from_value(serde_json::json!("nonsense"));
        assert!(
            result.is_err(),
            "a malformed reference in config must fail at load time"
        );
    }

    #[test]
    fn test_params_default_to_no_overrides() {
        let params = ModelParams::new();
        assert_eq!(params.temperature, None, "temperature should be unset");
        assert_eq!(params.max_tokens, None, "max tokens should be unset");
    }

    #[test]
    fn test_params_omit_unset_fields_when_serialized() {
        let params = ModelParams::new().with_temperature(1.0);
        let encoded = serde_json::to_value(&params).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!({"temperature": 1.0}),
            "unset knobs must not be sent as nulls, which some providers reject"
        );
    }

    #[test]
    fn test_params_builders_accumulate() {
        let params = ModelParams::new()
            .with_temperature(0.5)
            .with_max_tokens(2048)
            .with_thinking_budget(1024);
        assert_eq!(params.temperature, Some(0.5), "temperature should be set");
        assert_eq!(params.max_tokens, Some(2048), "max tokens should be set");
        assert_eq!(
            params.thinking_budget,
            Some(1024),
            "thinking budget should be set"
        );
    }
}
