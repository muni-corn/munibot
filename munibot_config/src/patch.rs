use serde::{Deserialize, Serialize};

use crate::{path::ConfigPath, value::ConfigValue};

/// One change within a `ConfigPatch`: set the value at `path` to `value`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigChange {
    pub path: ConfigPath,
    pub value: ConfigValue,
}

/// A set of changes to apply on top of a provider's current configuration.
///
/// Patches, rather than whole replacement objects, are what `ConfigValues`
/// being a flat map buys: the gui only has to send paths that actually
/// changed, and `Configurable::with_patch` only has to touch those.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigPatch {
    pub changes: Vec<ConfigChange>,
}

impl ConfigPatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a change, returning the extended patch.
    pub fn with(mut self, path: ConfigPath, value: ConfigValue) -> Self {
        self.changes.push(ConfigChange { path, value });
        self
    }
}

/// A single field-level problem found while applying a patch or validating
/// a configuration, e.g. an unparsable reference or a value outside a
/// field's declared bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldError {
    pub path: ConfigPath,
    pub message: String,
}

impl FieldError {
    pub fn new(path: ConfigPath, message: impl Into<String>) -> Self {
        Self {
            path,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigPatch, FieldError};
    use crate::{path::ConfigPath, value::ConfigValue};

    #[test]
    fn test_empty_patch_has_no_changes() {
        assert!(ConfigPatch::new().changes.is_empty());
    }

    #[test]
    fn test_with_appends_a_change() {
        let patch = ConfigPatch::new().with(
            ConfigPath::root().field("channel"),
            ConfigValue::Ref("1".to_owned()),
        );
        assert_eq!(patch.changes.len(), 1);
        assert_eq!(patch.changes[0].path, ConfigPath::root().field("channel"));
        assert_eq!(patch.changes[0].value, ConfigValue::Ref("1".to_owned()));
    }

    #[test]
    fn test_config_patch_roundtrips_through_json() {
        let patch = ConfigPatch::new()
            .with(ConfigPath::root().field("enabled"), ConfigValue::Bool(true))
            .with(
                ConfigPath::root().field("channel"),
                ConfigValue::Ref("123".to_owned()),
            );
        let json = serde_json::to_string(&patch).expect("serialize");
        let parsed: ConfigPatch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, patch);
    }

    #[test]
    fn test_field_error_roundtrips_through_json() {
        let error = FieldError::new(
            ConfigPath::root().field("channel"),
            "not a valid channel id",
        );
        let json = serde_json::to_string(&error).expect("serialize");
        let parsed: FieldError = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, error);
    }
}
