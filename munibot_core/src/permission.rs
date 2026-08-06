//! A capability a munibot user may be granted.
//!
//! Checked via `axum_session_auth::HasPermission` at the API layer
//! (`munibot_api::auth::server::User::has`), which compares plain strings -
//! this enum exists so every permission has exactly one canonical string
//! token, defined here, rather than raw string literals scattered across
//! every call site that grants or checks one.

use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display, EnumString};

/// One capability, serialized (to config, and to the `user_permissions`
/// table) as its snake_case string form - `Permission::Operator` becomes
/// `"operator"`.
///
/// `Display`/`FromStr` (via `strum`) are the single source of truth for that
/// string form; `Serialize`/`Deserialize` below are a thin bridge onto them
/// rather than a second, independently-maintained mapping.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum Permission {
    /// Can see aggregate, service-wide AI usage and spend - not scoped to
    /// any one guild, conversation, or user.
    Operator,
}

impl Serialize for Permission {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw)
            .map_err(|_| serde::de::Error::custom(format!("'{raw}' isn't a known permission")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_displays_as_snake_case() {
        assert_eq!(Permission::Operator.to_string(), "operator");
    }

    #[test]
    fn test_operator_parses_from_its_own_display_form() {
        assert_eq!(
            Permission::from_str("operator").unwrap(),
            Permission::Operator
        );
    }

    #[test]
    fn test_parsing_an_unknown_token_fails() {
        assert!(Permission::from_str("wizard").is_err());
    }

    #[test]
    fn test_serializes_as_a_bare_snake_case_string() {
        let json = serde_json::to_string(&Permission::Operator).unwrap();
        assert_eq!(json, "\"operator\"");
    }

    #[test]
    fn test_deserializes_from_its_own_serialized_form() {
        let permission: Permission = serde_json::from_str("\"operator\"").unwrap();
        assert_eq!(permission, Permission::Operator);
    }

    #[test]
    fn test_deserializing_an_unknown_token_fails() {
        let result: Result<Permission, _> = serde_json::from_str("\"wizard\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_toml_roundtrips_inside_a_config_shaped_struct() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            permission: Permission,
        }

        let wrapper = Wrapper {
            permission: Permission::Operator,
        };
        let toml_str = toml::to_string(&wrapper).unwrap();
        assert_eq!(toml_str, "permission = \"operator\"\n");

        let parsed: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.permission, Permission::Operator);
    }
}
