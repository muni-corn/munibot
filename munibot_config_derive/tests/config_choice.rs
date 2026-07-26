//! Compiles and exercises `#[derive(ConfigChoice)]`, verifying it produces
//! a working `ConfigChoice` + `ConfigField` + `Serialize`/`Deserialize`
//! implementation, and that ids/labels default sensibly from variant names.

use munibot_config::{ChoiceOption, ConfigChoice, ConfigField, ConfigValue, FieldKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, ConfigChoice)]
enum CleanupMode {
    Always,
    #[default]
    AfterSilence,
}

#[test]
fn test_choice_options_default_ids_and_labels_from_variant_names() {
    let options = CleanupMode::choice_options();
    assert_eq!(options, vec![
        ChoiceOption {
            id: "always".to_owned(),
            label: "always".to_owned(),
        },
        ChoiceOption {
            id: "after_silence".to_owned(),
            label: "after silence".to_owned(),
        },
    ]);
}

#[test]
fn test_choice_id_and_from_choice_id_roundtrip() {
    assert_eq!(CleanupMode::Always.choice_id(), "always");
    assert_eq!(
        CleanupMode::from_choice_id("after_silence"),
        Some(CleanupMode::AfterSilence)
    );
}

#[test]
fn test_from_choice_id_rejects_an_unknown_id() {
    assert_eq!(CleanupMode::from_choice_id("bogus"), None);
}

#[test]
fn test_config_field_impl_uses_the_choice_kind_and_ids() {
    let FieldKind::Choice { options } = CleanupMode::field_kind() else {
        panic!("expected FieldKind::Choice");
    };
    assert_eq!(options.len(), 2);

    assert_eq!(
        CleanupMode::Always.to_value(),
        ConfigValue::Choice("always".to_owned())
    );
    assert_eq!(
        CleanupMode::from_value(&ConfigValue::Choice("after_silence".to_owned())),
        Ok(CleanupMode::AfterSilence)
    );
}

#[test]
fn test_serialize_and_deserialize_use_choice_ids_not_rust_variant_names() {
    let json = serde_json::to_string(&CleanupMode::AfterSilence).expect("serialize");
    assert_eq!(json, "\"after_silence\"");

    let parsed: CleanupMode = serde_json::from_str("\"always\"").expect("deserialize");
    assert_eq!(parsed, CleanupMode::Always);
}

#[test]
fn test_deserialize_rejects_an_unknown_id() {
    let result: Result<CleanupMode, _> = serde_json::from_str("\"bogus\"");
    assert!(result.is_err());
}

#[test]
fn test_explicit_id_and_label_override_the_defaults() {
    #[derive(Clone, Copy, Debug, Default, PartialEq, ConfigChoice)]
    enum X {
        #[default]
        #[config(id = "custom_id", label = "custom label")]
        SomeVariant,
    }

    let options = X::choice_options();
    assert_eq!(options[0].id, "custom_id");
    assert_eq!(options[0].label, "custom label");
}
