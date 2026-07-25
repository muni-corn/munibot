use serde::{Deserialize, Serialize};

use crate::scope::ScopeKind;

/// Static metadata describing a configuration provider, independent of any
/// particular scope's values.
///
/// This is what the gui uses to list available providers (e.g. "server
/// logging", "autodelete") for a guild, before the user has picked one to
/// configure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Stable identifier used in urls and storage. Changing this for an
    /// existing provider orphans any settings already saved under the old
    /// id, so treat it like a table name.
    pub id: String,

    pub label: String,

    pub description: Option<String>,

    /// Phosphor duotone icon class name, e.g. "ph-megaphone-duotone", to
    /// match the icon font already loaded by the gui (see
    /// `munibot_gui/src/app.rs`).
    pub icon: Option<String>,

    /// The scopes this provider can be configured under. Checked by the
    /// server before dispatching to the provider.
    pub scopes: Vec<ScopeKind>,
}

#[cfg(test)]
mod tests {
    use super::ProviderDescriptor;
    use crate::scope::ScopeKind;

    #[test]
    fn test_provider_descriptor_roundtrips_through_json() {
        let descriptor = ProviderDescriptor {
            id: "logging".to_owned(),
            label: "server logging".to_owned(),
            description: Some("post an audit trail to a channel.".to_owned()),
            icon: Some("ph-megaphone-duotone".to_owned()),
            scopes: vec![ScopeKind::DiscordGuild],
        };

        let json = serde_json::to_string(&descriptor).expect("serialize");
        let parsed: ProviderDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, descriptor);
    }
}
