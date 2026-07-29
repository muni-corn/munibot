use std::{collections::HashMap, sync::LazyLock};

use serde::Deserialize;

use crate::types::{Cost, ModelRef, Usage};

/// Per-million-token prices for one model, in whole dollars.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    #[serde(default)]
    pub cache_read_per_mtok: f64,
    #[serde(default)]
    pub cache_write_per_mtok: f64,
}

impl Pricing {
    /// Estimates the cost of one usage record under this pricing.
    ///
    /// Reasoning tokens are billed at the output rate, since every provider
    /// munibot targets bills them that way - see the note on
    /// [`Usage::reasoning_tokens`].
    pub fn estimate(&self, usage: &Usage) -> Cost {
        let billed_output_tokens = usage.output_tokens + usage.reasoning_tokens;

        let dollars = per_token_cost(usage.input_tokens, self.input_per_mtok)
            + per_token_cost(billed_output_tokens, self.output_per_mtok)
            + per_token_cost(usage.cache_read_tokens, self.cache_read_per_mtok)
            + per_token_cost(usage.cache_write_tokens, self.cache_write_per_mtok);

        Cost::from_dollars(dollars)
    }
}

fn per_token_cost(tokens: u64, price_per_mtok: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price_per_mtok
}

/// The embedded pricing table source. A build-time asset, not user input - see
/// `pricing.toml` for the update process.
static TABLE_SOURCE: &str = include_str!("pricing.toml");

static TABLE: LazyLock<HashMap<String, Pricing>> = LazyLock::new(|| {
    toml::from_str(TABLE_SOURCE)
        .expect("munibot_ai's embedded pricing.toml must parse; this is a build-time asset")
});

/// Estimates the cost of a usage record for a given model.
///
/// A model missing from the pricing table yields [`Cost::ZERO`] and logs a
/// warning, rather than failing the turn over an accounting gap - an unpriced
/// model is a configuration omission worth fixing, not a reason to refuse to
/// answer.
pub fn estimate_cost(model: &ModelRef, usage: &Usage) -> Cost {
    match TABLE.get(&model.to_string()) {
        Some(pricing) => pricing.estimate(usage),
        None => {
            tracing::warn!(model = %model, "no pricing entry for this model; cost will read as zero");
            Cost::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_table_parses() {
        // exercised implicitly by every other test through TABLE, but a dedicated test
        // names the failure clearly if the embedded file is ever hand-edited
        // into invalid TOML
        assert!(
            !TABLE.is_empty(),
            "the embedded pricing table should parse into at least one entry"
        );
    }

    #[test]
    fn test_known_model_estimates_input_and_output_cost() {
        let model = ModelRef::new("anthropic", "claude-opus-5");
        let usage = Usage::new(1_000_000, 1_000_000);

        let cost = estimate_cost(&model, &usage);

        // one million input tokens at $15/mtok plus one million output tokens at
        // $75/mtok
        assert_eq!(cost, Cost::from_dollars(90.0));
    }

    #[test]
    fn test_reasoning_tokens_are_billed_at_the_output_rate() {
        let model = ModelRef::new("anthropic", "claude-opus-5");
        let with_reasoning = Usage {
            reasoning_tokens: 1_000_000,
            ..Usage::default()
        };
        let as_output = Usage::new(0, 1_000_000);

        assert_eq!(
            estimate_cost(&model, &with_reasoning),
            estimate_cost(&model, &as_output),
            "a million reasoning tokens should cost exactly what a million output tokens costs"
        );
    }

    #[test]
    fn test_cache_tokens_are_priced_separately_from_input() {
        let model = ModelRef::new("anthropic", "claude-opus-5");
        let cache_read = Usage {
            cache_read_tokens: 1_000_000,
            ..Usage::default()
        };
        let plain_input = Usage::new(1_000_000, 0);

        assert_ne!(
            estimate_cost(&model, &cache_read),
            estimate_cost(&model, &plain_input),
            "cache reads are billed at a discount, not the full input rate"
        );
        // one million cache-read tokens at $1.5/mtok
        assert_eq!(estimate_cost(&model, &cache_read), Cost::from_dollars(1.5));
    }

    #[test]
    fn test_unknown_model_yields_zero_cost() {
        let model = ModelRef::new("anthropic", "some-future-model-not-in-the-table");
        let usage = Usage::new(1_000_000, 1_000_000);

        assert_eq!(
            estimate_cost(&model, &usage),
            Cost::ZERO,
            "an unpriced model should read as zero cost rather than fail the turn"
        );
    }

    #[test]
    fn test_pricing_defaults_missing_cache_fields_to_zero() {
        let pricing: Pricing = toml::from_str(
            r#"
            input_per_mtok = 1.0
            output_per_mtok = 2.0
            "#,
        )
        .expect("should parse without cache fields");

        assert_eq!(pricing.cache_read_per_mtok, 0.0);
        assert_eq!(pricing.cache_write_per_mtok, 0.0);
    }

    #[test]
    fn test_zero_usage_is_zero_cost() {
        let model = ModelRef::new("anthropic", "claude-opus-5");
        assert_eq!(estimate_cost(&model, &Usage::default()), Cost::ZERO);
    }
}
