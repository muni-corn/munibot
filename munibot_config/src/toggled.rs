use serde::{Deserialize, Serialize};

/// Wraps a settings struct with an "enabled" switch, e.g. `logging` having
/// both an on/off toggle and the settings (like which channel) that only
/// matter while it's on.
///
/// The `Configurable` derive maps a `Toggled<T>` field to a `Section` whose
/// `toggle` is the `enabled` field and whose `children` are `T`'s fields,
/// with those children's `visible_when` wired to `enabled` automatically.
/// This is what removes the need for a stringly-typed `visible_when`
/// attribute in the common enable-this-to-see-more case -- see
/// docs/plans/gui-configuration.md section 6.1.
///
/// `settings` is flattened so `Toggled<T>`'s json shape is `{"enabled":
/// ..., ...T's own fields}` rather than nesting `T` under its own key,
/// matching the single json object a `Section`'s fields occupy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Toggled<T> {
    pub enabled: bool,

    #[serde(flatten)]
    pub settings: T,
}

impl<T> Toggled<T> {
    pub fn new(enabled: bool, settings: T) -> Self {
        Self { enabled, settings }
    }

    /// `settings` if `enabled`, otherwise `None`. The usual way a provider
    /// reads a toggled setting: disabled and "no channel configured" should
    /// behave identically.
    pub fn if_enabled(&self) -> Option<&T> {
        self.enabled.then_some(&self.settings)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::Toggled;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct LoggingSettings {
        channel: Option<u64>,
    }

    #[test]
    fn test_toggled_default_is_disabled_with_default_settings() {
        let toggled = Toggled::<LoggingSettings>::default();
        assert!(!toggled.enabled);
        assert_eq!(toggled.settings, LoggingSettings::default());
    }

    #[test]
    fn test_if_enabled_returns_none_when_disabled() {
        let toggled = Toggled::new(false, LoggingSettings { channel: Some(123) });
        assert_eq!(toggled.if_enabled(), None);
    }

    #[test]
    fn test_if_enabled_returns_settings_when_enabled() {
        let settings = LoggingSettings { channel: Some(123) };
        let toggled = Toggled::new(true, settings.clone());
        assert_eq!(toggled.if_enabled(), Some(&settings));
    }

    #[test]
    fn test_toggled_serializes_with_settings_flattened() {
        let toggled = Toggled::new(true, LoggingSettings { channel: Some(123) });
        let json = serde_json::to_string(&toggled).expect("serialize");
        assert_eq!(json, r#"{"enabled":true,"channel":123}"#);
    }

    #[test]
    fn test_toggled_deserializes_from_flattened_json() {
        let toggled: Toggled<LoggingSettings> =
            serde_json::from_str(r#"{"enabled":true,"channel":123}"#).expect("deserialize");
        assert_eq!(
            toggled,
            Toggled::new(true, LoggingSettings { channel: Some(123) })
        );
    }
}
