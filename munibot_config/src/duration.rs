use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    field::FieldKind,
    field_value::{ConfigField, ValueError},
    value::ConfigValue,
};

/// A `std::time::Duration`, as a leaf value in a `Configurable` struct.
///
/// Stored (and serialized) as whole seconds. Sub-second precision has no
/// use case in any of munibot's settings (e.g. autodelete lifetimes), and
/// dropping it keeps the type simple.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SettingDuration(#[serde(with = "duration_secs")] Duration);

impl SettingDuration {
    pub fn new(duration: Duration) -> Self {
        Self(duration)
    }

    pub fn get(self) -> Duration {
        self.0
    }
}

impl From<Duration> for SettingDuration {
    fn from(duration: Duration) -> Self {
        Self(duration)
    }
}

impl From<SettingDuration> for Duration {
    fn from(setting: SettingDuration) -> Self {
        setting.0
    }
}

mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_secs)
    }
}

impl ConfigField for SettingDuration {
    fn field_kind() -> FieldKind {
        FieldKind::Duration {
            min: None,
            max: None,
        }
    }

    fn to_value(&self) -> ConfigValue {
        ConfigValue::Duration(self.0.as_secs())
    }

    fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
        match value {
            ConfigValue::Duration(secs) => Ok(Self(Duration::from_secs(*secs))),
            other => Err(ValueError::WrongKind {
                expected: "duration",
                found: other.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SettingDuration;
    use crate::{field::FieldKind, field_value::ConfigField, value::ConfigValue};

    #[test]
    fn test_setting_duration_field_kind_has_no_default_bounds() {
        assert_eq!(SettingDuration::field_kind(), FieldKind::Duration {
            min: None,
            max: None
        });
    }

    #[test]
    fn test_setting_duration_roundtrips_through_config_value() {
        let duration = SettingDuration::new(Duration::from_secs(3600));
        let value = duration.to_value();
        assert_eq!(value, ConfigValue::Duration(3600));
        assert_eq!(SettingDuration::from_value(&value), Ok(duration));
    }

    #[test]
    fn test_setting_duration_serializes_as_seconds() {
        let duration = SettingDuration::new(Duration::from_secs(86400));
        let json = serde_json::to_string(&duration).expect("serialize");
        assert_eq!(json, "86400");
    }

    #[test]
    fn test_setting_duration_truncates_sub_second_precision() {
        let duration = SettingDuration::new(Duration::from_millis(1500));
        assert_eq!(duration.to_value(), ConfigValue::Duration(1));
    }

    #[test]
    fn test_setting_duration_converts_from_and_into_std_duration() {
        let std_duration = Duration::from_secs(60);
        let setting: SettingDuration = std_duration.into();
        let back: Duration = setting.into();
        assert_eq!(back, std_duration);
    }
}
