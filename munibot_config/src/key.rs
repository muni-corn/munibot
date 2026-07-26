use crate::{
    field::KeyKind,
    field_value::ValueError,
    refs::{ChannelRef, RoleRef},
    value::ConfigValue,
};

/// A type that can key a `Collection` field, e.g. `BTreeMap<K, V>` where
/// `K: ConfigKey`, identifying one instance among many by an external id.
///
/// Only `ChannelRef` and `RoleRef` implement this: a `Collection`'s key is
/// always an external reference (see `KeyKind`), never an arbitrary string
/// or number, so there's always a well-defined `ConfigValue::Ref` on the
/// wire for it.
pub trait ConfigKey: Ord + Sized {
    fn key_kind() -> KeyKind;
    fn to_key_string(&self) -> String;
    fn from_key_string(s: &str) -> Result<Self, ValueError>;
}

impl ConfigKey for ChannelRef {
    fn key_kind() -> KeyKind {
        KeyKind::DiscordChannel
    }

    fn to_key_string(&self) -> String {
        self.0.to_string()
    }

    fn from_key_string(s: &str) -> Result<Self, ValueError> {
        s.parse().map(Self).map_err(|_| ValueError::WrongKind {
            expected: "discord channel",
            found: ConfigValue::Ref(s.to_owned()),
        })
    }
}

impl ConfigKey for RoleRef {
    fn key_kind() -> KeyKind {
        KeyKind::DiscordRole
    }

    fn to_key_string(&self) -> String {
        self.0.to_string()
    }

    fn from_key_string(s: &str) -> Result<Self, ValueError> {
        s.parse().map(Self).map_err(|_| ValueError::WrongKind {
            expected: "discord role",
            found: ConfigValue::Ref(s.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigKey;
    use crate::{field::KeyKind, refs::ChannelRef};

    #[test]
    fn test_channel_ref_key_kind_is_discord_channel() {
        assert_eq!(ChannelRef::key_kind(), KeyKind::DiscordChannel);
    }

    #[test]
    fn test_channel_ref_key_string_roundtrips() {
        let channel = ChannelRef(6437);
        let key = channel.to_key_string();
        assert_eq!(key, "6437");
        assert_eq!(ChannelRef::from_key_string(&key), Ok(channel));
    }

    #[test]
    fn test_channel_ref_from_key_string_rejects_non_numeric_input() {
        assert!(ChannelRef::from_key_string("not a number").is_err());
    }
}
