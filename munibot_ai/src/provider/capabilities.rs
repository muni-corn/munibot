use std::{collections::HashMap, sync::LazyLock};

use serde::Deserialize;

use crate::types::ModelRef;

/// Per-model facts about what a model can do, beyond price.
///
/// Its own table, table format, and lookup mirror [`super::Pricing`]
/// deliberately - both are configuration facts about a `"provider:model"`
/// string, loaded once at startup and looked up by [`ModelRef::to_string`].
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Capabilities {
    /// Whether this model accepts image input at all.
    #[serde(default)]
    pub vision: bool,
}

/// The embedded capabilities table source. A build-time asset, not user
/// input - see `capabilities.toml` for the update process.
static TABLE_SOURCE: &str = include_str!("capabilities.toml");

static TABLE: LazyLock<HashMap<String, Capabilities>> = LazyLock::new(|| {
    toml::from_str(TABLE_SOURCE)
        .expect("munibot_ai's embedded capabilities.toml must parse; this is a build-time asset")
});

/// Whether `model` accepts image input.
///
/// A model missing from the table reads as `false`, the opposite default
/// from [`super::estimate_cost`]'s zero-cost fallback, and deliberately so:
/// an unpriced model undercounting a bill is a minor accounting gap, but an
/// unlisted model silently answering as though it had looked at an image it
/// never saw is exactly the failure mode this table exists to prevent - see
/// [`crate::persona::Persona::ensure_can_see`] for where that refusal
/// actually happens.
pub fn supports_vision(model: &ModelRef) -> bool {
    TABLE
        .get(&model.to_string())
        .is_some_and(|capabilities| capabilities.vision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_table_parses() {
        assert!(
            !TABLE.is_empty(),
            "the embedded capabilities table should parse into at least one entry"
        );
    }

    #[test]
    fn test_known_vision_model_supports_vision() {
        let model = ModelRef::new("anthropic", "claude-opus-5");
        assert!(supports_vision(&model));
    }

    #[test]
    fn test_unknown_model_does_not_support_vision() {
        let model = ModelRef::new("anthropic", "some-future-model-not-in-the-table");
        assert!(
            !supports_vision(&model),
            "a model missing from the table should read as unable to see, not able to"
        );
    }
}
