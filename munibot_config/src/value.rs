use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::path::ConfigPath;

/// A single configuration value, as it travels over the wire between the
/// server and the wasm gui client.
///
/// Storage uses a different, untagged representation -- see
/// `munibot_core`'s config store -- because the schema already knows each
/// field's type, so a tag would be redundant there. The wire
/// representation stays tagged because it has no schema of its own to lean
/// on for `Deserialize`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
    /// No value has been set; the field's default applies.
    #[default]
    Unset,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    /// Seconds.
    Duration(u64),
    /// The id of a chosen `ChoiceOption`.
    Choice(String),
    MultiChoice(Vec<String>),
    /// An external id, e.g. a discord snowflake, stored as text -- matching
    /// how discord ids already travel over json elsewhere in munibot (see
    /// `GuildSummary::id`), and safe from precision loss since neither end
    /// of this wire is a javascript float.
    Ref(String),
    List(Vec<ConfigValue>),
}

/// A flat map from a path to its value, covering an entire provider's
/// configuration (or a patch to it) regardless of how deeply nested the
/// schema is.
pub type ConfigValues = BTreeMap<ConfigPath, ConfigValue>;

#[cfg(test)]
mod tests {
    use super::ConfigValue;

    #[test]
    fn test_default_config_value_is_unset() {
        assert_eq!(ConfigValue::default(), ConfigValue::Unset);
    }

    #[test]
    fn test_config_value_roundtrips_through_json() {
        let values = [
            ConfigValue::Unset,
            ConfigValue::Bool(true),
            ConfigValue::Int(-7),
            ConfigValue::Float(1.5),
            ConfigValue::Text("hi :3".to_owned()),
            ConfigValue::Duration(3600),
            ConfigValue::Choice("after_silence".to_owned()),
            ConfigValue::MultiChoice(vec!["a".to_owned(), "b".to_owned()]),
            ConfigValue::Ref("123456789012345678".to_owned()),
            ConfigValue::List(vec![ConfigValue::Int(1), ConfigValue::Int(2)]),
        ];

        for value in values {
            let json = serde_json::to_string(&value).expect("serialize");
            let parsed: ConfigValue = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, value, "roundtrip failed for {value:?}");
        }
    }

    #[test]
    fn test_ref_value_preserves_full_snowflake_precision() {
        // a discord snowflake can exceed 2^53, which would lose precision if
        // it were ever represented as an f64 (e.g. a javascript number).
        // storing it as text sidesteps that entirely.
        let snowflake = "18446744073709551615";
        let value = ConfigValue::Ref(snowflake.to_owned());
        let json = serde_json::to_string(&value).expect("serialize");
        let parsed: ConfigValue = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, ConfigValue::Ref(snowflake.to_owned()));
    }
}
