use serde::{Deserialize, Serialize};

use crate::{
    field::{FieldKind, RefKind},
    field_value::{ConfigField, ValueError},
    value::ConfigValue,
};

// a snowflake, stored as a plain u64 in the struct itself (unlike
// ConfigValue::Ref, which carries it as text -- see value.rs for why the
// wire representation is text). the struct's own serde impl only ever
// round-trips through this crate's serde_json flatten/unflatten, which is
// rust-side on both ends, so there's no javascript-precision boundary to
// protect against here.
macro_rules! discord_ref {
    ($name:ident, $ref_kind:expr, $expected:literal) => {
        #[doc = concat!("A ", $expected, " id, as a leaf value in a `Configurable` struct.")]
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl ConfigField for $name {
            fn field_kind() -> FieldKind {
                FieldKind::Ref { kind: $ref_kind }
            }

            fn to_value(&self) -> ConfigValue {
                ConfigValue::Ref(self.0.to_string())
            }

            fn from_value(value: &ConfigValue) -> Result<Self, ValueError> {
                match value {
                    ConfigValue::Ref(s) => s.parse().map(Self).map_err(|_| ValueError::WrongKind {
                        expected: $expected,
                        found: value.clone(),
                    }),
                    other => Err(ValueError::WrongKind {
                        expected: $expected,
                        found: other.clone(),
                    }),
                }
            }
        }
    };
}

discord_ref!(ChannelRef, RefKind::DiscordChannel, "discord channel");
discord_ref!(RoleRef, RefKind::DiscordRole, "discord role");
discord_ref!(UserRef, RefKind::DiscordUser, "discord user");

#[cfg(test)]
mod tests {
    use super::{ChannelRef, RoleRef, UserRef};
    use crate::{
        field::{FieldKind, RefKind},
        field_value::{ConfigField, ValueError},
        value::ConfigValue,
    };

    #[test]
    fn test_channel_ref_field_kind_is_discord_channel() {
        assert_eq!(ChannelRef::field_kind(), FieldKind::Ref {
            kind: RefKind::DiscordChannel
        });
    }

    #[test]
    fn test_channel_ref_roundtrips_through_config_value() {
        let channel = ChannelRef(123456789012345678);
        let value = channel.to_value();
        assert_eq!(value, ConfigValue::Ref("123456789012345678".to_owned()));
        assert_eq!(ChannelRef::from_value(&value), Ok(channel));
    }

    #[test]
    fn test_role_ref_and_user_ref_use_their_own_ref_kind() {
        assert_eq!(RoleRef::field_kind(), FieldKind::Ref {
            kind: RefKind::DiscordRole
        });
        assert_eq!(UserRef::field_kind(), FieldKind::Ref {
            kind: RefKind::DiscordUser
        });
    }

    #[test]
    fn test_from_value_rejects_non_numeric_ref_text() {
        assert_eq!(
            ChannelRef::from_value(&ConfigValue::Ref("not a number".to_owned())),
            Err(ValueError::WrongKind {
                expected: "discord channel",
                found: ConfigValue::Ref("not a number".to_owned()),
            })
        );
    }

    #[test]
    fn test_from_value_rejects_wrong_config_value_variant() {
        assert_eq!(
            ChannelRef::from_value(&ConfigValue::Bool(true)),
            Err(ValueError::WrongKind {
                expected: "discord channel",
                found: ConfigValue::Bool(true),
            })
        );
    }

    #[test]
    fn test_channel_ref_serializes_as_a_plain_json_number() {
        let json = serde_json::to_string(&ChannelRef(42)).expect("serialize");
        assert_eq!(json, "42");
    }
}
