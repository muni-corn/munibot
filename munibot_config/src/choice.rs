use crate::{
    field::{ChoiceOption, FieldKind},
    field_value::ValueError,
    value::ConfigValue,
};

/// A fixed set of named options a field can be set to, e.g. an enum like
/// `AutoDeleteMode`.
///
/// `#[derive(ConfigChoice)]` (`munibot_config_derive`, phase 2) implements
/// this for an enum of unit variants, and generates a matching `impl
/// ConfigField` using the free functions below -- see
/// `choice_field_kind`/`choice_to_value`/`choice_from_value`. There's no
/// blanket `impl<T: ConfigChoice> ConfigField for T` here on purpose: it
/// would force every future `ConfigField` impl in this crate to prove it
/// can never also implement `ConfigChoice`, for no real benefit over the
/// derive generating the (tiny) impl directly.
pub trait ConfigChoice: Sized {
    /// The full set of options, in declaration order.
    fn choice_options() -> Vec<ChoiceOption>;

    /// The id of this value, matching one of `choice_options()`.
    fn choice_id(&self) -> String;

    /// Parses a value from an option id, or `None` if it doesn't match any
    /// option currently in `choice_options()`.
    fn from_choice_id(id: &str) -> Option<Self>;
}

/// Building block for a generated `ConfigField::field_kind` impl.
pub fn choice_field_kind<T: ConfigChoice>() -> FieldKind {
    FieldKind::Choice {
        options: T::choice_options(),
    }
}

/// Building block for a generated `ConfigField::to_value` impl.
pub fn choice_to_value<T: ConfigChoice>(value: &T) -> ConfigValue {
    ConfigValue::Choice(value.choice_id())
}

/// Building block for a generated `ConfigField::from_value` impl.
pub fn choice_from_value<T: ConfigChoice>(value: &ConfigValue) -> Result<T, ValueError> {
    match value {
        ConfigValue::Choice(id) => T::from_choice_id(id).ok_or_else(|| ValueError::WrongKind {
            expected: "a known choice id",
            found: value.clone(),
        }),
        other => Err(ValueError::WrongKind {
            expected: "choice",
            found: other.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigChoice, choice_field_kind, choice_from_value, choice_to_value};
    use crate::{
        field::{ChoiceOption, FieldKind},
        value::ConfigValue,
    };

    // stands in for what `#[derive(ConfigChoice)]` will generate in phase 2.
    #[derive(Debug, PartialEq)]
    enum CleanupMode {
        Always,
        AfterSilence,
    }

    impl ConfigChoice for CleanupMode {
        fn choice_options() -> Vec<ChoiceOption> {
            vec![
                ChoiceOption {
                    id: "always".to_owned(),
                    label: "always".to_owned(),
                },
                ChoiceOption {
                    id: "after_silence".to_owned(),
                    label: "after silence".to_owned(),
                },
            ]
        }

        fn choice_id(&self) -> String {
            match self {
                Self::Always => "always".to_owned(),
                Self::AfterSilence => "after_silence".to_owned(),
            }
        }

        fn from_choice_id(id: &str) -> Option<Self> {
            match id {
                "always" => Some(Self::Always),
                "after_silence" => Some(Self::AfterSilence),
                _ => None,
            }
        }
    }

    #[test]
    fn test_choice_field_kind_lists_all_options() {
        let FieldKind::Choice { options } = choice_field_kind::<CleanupMode>() else {
            panic!("expected FieldKind::Choice");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "always");
        assert_eq!(options[1].id, "after_silence");
    }

    #[test]
    fn test_choice_to_value_uses_the_variant_id() {
        assert_eq!(
            choice_to_value(&CleanupMode::AfterSilence),
            ConfigValue::Choice("after_silence".to_owned())
        );
    }

    #[test]
    fn test_choice_from_value_parses_a_known_id() {
        assert_eq!(
            choice_from_value::<CleanupMode>(&ConfigValue::Choice("always".to_owned())),
            Ok(CleanupMode::Always)
        );
    }

    #[test]
    fn test_choice_from_value_rejects_an_unknown_id() {
        // unlike AutoDeleteMode::from_db_str's silent fallback (see
        // docs/plans/gui-configuration.md section 13), an unrecognized
        // choice id is a real error, not a default -- a bad patch should
        // surface as a field error, not silently coerce to some variant.
        assert!(
            choice_from_value::<CleanupMode>(&ConfigValue::Choice("bogus".to_owned())).is_err()
        );
    }

    #[test]
    fn test_choice_from_value_rejects_the_wrong_config_value_variant() {
        assert!(choice_from_value::<CleanupMode>(&ConfigValue::Bool(true)).is_err());
    }
}
