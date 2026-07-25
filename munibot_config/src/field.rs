use serde::{Deserialize, Serialize};

use crate::value::ConfigValue;

/// One selectable value of a `Choice` or `MultiChoice` field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// Stable identifier stored as the value; never shown to the user.
    pub id: String,
    pub label: String,
}

/// What kind of external reference a `Ref` field points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    DiscordChannel,
    DiscordRole,
    DiscordUser,
}

/// What kind of external reference keys a `Collection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyKind {
    DiscordChannel,
    DiscordRole,
}

/// The type and constraints of a single configuration field.
///
/// Note: this deviates from docs/plans/gui-configuration.md section 5.3 by
/// adding `Float`. The plan's `ConfigValue` already had a `Float` variant
/// with no matching `FieldKind`, which would have made it unreachable from
/// any real schema; see docs/notes/config-schema-implementation-notes.md.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldKind {
    Bool,
    Int {
        min: Option<i64>,
        max: Option<i64>,
        step: i64,
        unit: Option<String>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
        unit: Option<String>,
    },
    Text {
        multiline: bool,
        max_len: Option<usize>,
        placeholder: Option<String>,
    },
    /// Seconds.
    Duration {
        min: Option<u64>,
        max: Option<u64>,
    },
    Choice {
        options: Vec<ChoiceOption>,
    },
    MultiChoice {
        options: Vec<ChoiceOption>,
    },
    Ref {
        kind: RefKind,
    },
    List {
        item: Box<FieldKind>,
        max_items: Option<usize>,
    },
    Color,
    Url,
}

/// A condition under which a field or a toggled section's children are
/// shown, evaluated against the current (possibly unsaved) values of its
/// siblings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    /// The id of a sibling field.
    pub field_id: String,
    pub equals: ConfigValue,
}

/// A single configuration field: its identity, type, and display metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Identifier unique among its siblings; becomes this field's last path
    /// segment.
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: FieldKind,
    pub default: ConfigValue,
    pub required: bool,
    /// Shown only when the referenced sibling field equals a given value.
    pub visible_when: Option<Condition>,
}

impl Field {
    /// Starts building a field with the given id, label, and kind. Other
    /// properties default to "not required, no description, unset default,
    /// always visible".
    ///
    /// This is a plain constructor, not a schema-assembly dsl -- the
    /// `Configurable` derive (see docs/plans/gui-configuration.md section 6)
    /// is what builds real schemas. These methods exist so the derive's
    /// generated code (and tests) have an ergonomic way to construct one
    /// `Field` at a time.
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            kind,
            default: ConfigValue::Unset,
            required: false,
            visible_when: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn default(mut self, default: ConfigValue) -> Self {
        self.default = default;
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn visible_when(mut self, field_id: impl Into<String>, equals: ConfigValue) -> Self {
        self.visible_when = Some(Condition {
            field_id: field_id.into(),
            equals,
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Field, FieldKind};
    use crate::value::ConfigValue;

    #[test]
    fn test_field_new_has_sensible_defaults() {
        let field = Field::new("channel", "log channel", FieldKind::Bool);
        assert_eq!(field.id, "channel");
        assert_eq!(field.label, "log channel");
        assert!(field.description.is_none());
        assert_eq!(field.default, ConfigValue::Unset);
        assert!(!field.required);
        assert!(field.visible_when.is_none());
    }

    #[test]
    fn test_field_builder_methods_set_expected_properties() {
        let field = Field::new("channel", "log channel", FieldKind::Bool)
            .description("where to post logs")
            .default(ConfigValue::Bool(true))
            .required()
            .visible_when("enabled", ConfigValue::Bool(true));

        assert_eq!(field.description.as_deref(), Some("where to post logs"));
        assert_eq!(field.default, ConfigValue::Bool(true));
        assert!(field.required);
        let condition = field.visible_when.expect("visible_when should be set");
        assert_eq!(condition.field_id, "enabled");
        assert_eq!(condition.equals, ConfigValue::Bool(true));
    }

    #[test]
    fn test_field_roundtrips_through_json() {
        let field = Field::new("duration", "message lifetime", FieldKind::Duration {
            min: Some(3600),
            max: None,
        })
        .default(ConfigValue::Duration(86400));

        let json = serde_json::to_string(&field).expect("serialize");
        let parsed: Field = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, field);
    }

    #[test]
    fn test_list_field_kind_roundtrips_through_json() {
        let kind = FieldKind::List {
            item: Box::new(FieldKind::Text {
                multiline: false,
                max_len: None,
                placeholder: None,
            }),
            max_items: Some(10),
        };
        let json = serde_json::to_string(&kind).expect("serialize");
        let parsed: FieldKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, kind);
    }
}
