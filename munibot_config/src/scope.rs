use serde::{Deserialize, Serialize};

/// Where a set of configuration values applies.
///
/// New variants can be added as munibot gains more integrations (twitch,
/// github) without touching existing providers -- each provider declares
/// which `ScopeKind`s it supports via `ProviderDescriptor::scopes`, and
/// storage is keyed per provider, so adding a scope is additive.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfigScope {
    /// A single discord server. `guild_id` is the snowflake as text,
    /// matching how discord ids already travel over json elsewhere in
    /// munibot (see `GuildSummary::id`).
    DiscordGuild { guild_id: String },
}

impl ConfigScope {
    /// The `ScopeKind` this scope belongs to, discarding the identifying
    /// data.
    pub fn kind(&self) -> ScopeKind {
        match self {
            Self::DiscordGuild { .. } => ScopeKind::DiscordGuild,
        }
    }
}

/// The kind of a `ConfigScope`, without the data that identifies a specific
/// instance of it. `ProviderDescriptor` uses this to declare which scopes a
/// provider supports without needing a real scope to point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeKind {
    DiscordGuild,
}

#[cfg(test)]
mod tests {
    use super::{ConfigScope, ScopeKind};

    #[test]
    fn test_discord_guild_scope_kind_is_discord_guild() {
        let scope = ConfigScope::DiscordGuild {
            guild_id: "123".to_owned(),
        };
        assert_eq!(scope.kind(), ScopeKind::DiscordGuild);
    }

    #[test]
    fn test_config_scope_roundtrips_through_json() {
        let scope = ConfigScope::DiscordGuild {
            guild_id: "123".to_owned(),
        };
        let json = serde_json::to_string(&scope).expect("serialize");
        let parsed: ConfigScope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, scope);
    }
}
