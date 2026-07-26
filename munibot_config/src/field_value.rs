use thiserror::Error;

use crate::{field::FieldKind, value::ConfigValue};

/// An error converting a `ConfigValue` back into a typed field value.
///
/// This should be rare in practice: by the time a `ConfigValue` reaches a
/// `ConfigField::from_value` call, it has already been coerced against the
/// field's `FieldKind` (see the `Configurable` derive's generated code), so
/// a mismatch here usually means the schema and the stored/patched value
/// have drifted.
#[derive(Debug, Error, PartialEq)]
pub enum ValueError {
    #[error("expected a {expected} value, found {found:?}")]
    WrongKind {
        expected: &'static str,
        found: ConfigValue,
    },
}

/// A Rust type that can appear as a leaf value in a `Configurable` struct.
///
/// Implementing this trait (rather than annotating a plain field with a
/// "kind" attribute) is what lets the `Configurable` derive infer each
/// field's `FieldKind` from its Rust type. Semantic newtypes built on top of
/// this trait -- `ChannelRef`, `SettingDuration`, and so on -- are what
/// carry a field's real meaning; see docs/plans/gui-configuration.md
/// section 5.4.
pub trait ConfigField: Sized {
    fn field_kind() -> FieldKind;
    fn to_value(&self) -> ConfigValue;
    fn from_value(value: &ConfigValue) -> Result<Self, ValueError>;
}

impl ConfigField for bool {
    fn field_kind() -> FieldKind {
        FieldKind::Bool
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::Bool(*self)
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Bool(b) => Ok(*b),
            other => Err(ValueError::WrongKind {
                expected: "bool",
                found: other.clone(),
            }),
        }
    }
}

impl ConfigField for i64 {
    fn field_kind() -> FieldKind {
        FieldKind::Int {
            min: None,
            max: None,
            step: 1,
            unit: None,
        }
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::Int(*self)
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Int(i) => Ok(*i),
            other => Err(ValueError::WrongKind {
                expected: "int",
                found: other.clone(),
            }),
        }
    }
}

impl ConfigField for f64 {
    fn field_kind() -> FieldKind {
        FieldKind::Float {
            min: None,
            max: None,
            step: 1.0,
            unit: None,
        }
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::Float(*self)
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Float(f) => Ok(*f),
            other => Err(ValueError::WrongKind {
                expected: "float",
                found: other.clone(),
            }),
        }
    }
}

impl ConfigField for String {
    fn field_kind() -> FieldKind {
        FieldKind::Text {
            multiline: false,
            max_len: None,
            placeholder: None,
        }
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::Text(self.clone())
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Text(s) => Ok(s.clone()),
            other => Err(ValueError::WrongKind {
                expected: "text",
                found: other.clone(),
            }),
        }
    }
}

impl<T: ConfigField> ConfigField for Option<T> {
    fn field_kind() -> FieldKind {
        T::field_kind()
    }

    fn to_value(&self) -> ConfigValue {
        match self {
            Some(t) => t.to_value(),
            None => ConfigValue::Unset,
        }
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Unset => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

impl<T: ConfigField> ConfigField for Vec<T> {
    fn field_kind() -> FieldKind {
        FieldKind::List {
            item: Box::new(T::field_kind()),
            max_items: None,
        }
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::List(self.iter().map(ConfigField::to_value).collect())
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            // an absent list is an empty one, not an error -- a Vec field
            // with no saved value yet should behave like Default::default().
            ConfigValue::Unset => Ok(Vec::new()),
            ConfigValue::List(items) => items.iter().map(T::from_value).collect(),
            other => Err(ValueError::WrongKind {
                expected: "list",
                found: other.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigField, ValueError};
    use crate::{field::FieldKind, value::ConfigValue};

    #[test]
    fn test_bool_roundtrips_through_config_value() {
        assert_eq!(true.to_value(), ConfigValue::Bool(true));
        assert_eq!(bool::from_value(&ConfigValue::Bool(true)), Ok(true));
        assert_eq!(bool::field_kind(), FieldKind::Bool);
    }

    #[test]
    fn test_i64_roundtrips_through_config_value() {
        assert_eq!(7i64.to_value(), ConfigValue::Int(7));
        assert_eq!(i64::from_value(&ConfigValue::Int(7)), Ok(7));
    }

    #[test]
    fn test_f64_roundtrips_through_config_value() {
        assert_eq!(1.5f64.to_value(), ConfigValue::Float(1.5));
        assert_eq!(f64::from_value(&ConfigValue::Float(1.5)), Ok(1.5));
    }

    #[test]
    fn test_string_roundtrips_through_config_value() {
        let s = "hi :3".to_owned();
        assert_eq!(s.to_value(), ConfigValue::Text(s.clone()));
        assert_eq!(String::from_value(&ConfigValue::Text(s.clone())), Ok(s));
    }

    #[test]
    fn test_from_value_reports_the_expected_kind_on_mismatch() {
        assert_eq!(
            bool::from_value(&ConfigValue::Int(1)),
            Err(ValueError::WrongKind {
                expected: "bool",
                found: ConfigValue::Int(1),
            })
        );
    }

    #[test]
    fn test_option_none_is_unset() {
        assert_eq!(None::<bool>.to_value(), ConfigValue::Unset);
        assert_eq!(Option::<bool>::from_value(&ConfigValue::Unset), Ok(None));
    }

    #[test]
    fn test_option_some_delegates_to_inner_type() {
        assert_eq!(Some(true).to_value(), ConfigValue::Bool(true));
        assert_eq!(
            Option::<bool>::from_value(&ConfigValue::Bool(true)),
            Ok(Some(true))
        );
    }

    #[test]
    fn test_vec_roundtrips_through_config_value() {
        let items = vec![1i64, 2, 3];
        assert_eq!(
            items.to_value(),
            ConfigValue::List(vec![
                ConfigValue::Int(1),
                ConfigValue::Int(2),
                ConfigValue::Int(3),
            ])
        );
        assert_eq!(
            Vec::<i64>::from_value(&ConfigValue::List(vec![
                ConfigValue::Int(1),
                ConfigValue::Int(2),
                ConfigValue::Int(3),
            ])),
            Ok(items)
        );
    }

    #[test]
    fn test_unset_vec_is_empty_rather_than_an_error() {
        assert_eq!(Vec::<i64>::from_value(&ConfigValue::Unset), Ok(Vec::new()));
    }
}
