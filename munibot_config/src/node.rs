use serde::{Deserialize, Serialize};

use crate::{
    descriptor::ProviderDescriptor,
    field::{Field, KeyKind},
};

/// A node in a provider's configuration tree.
///
/// A provider's tree shape is entirely static; only `ConfigValues` (and,
/// for `Collection`s, which keys currently exist within it) vary per scope.
/// This is what lets the gui render a schema generically, and what lets
/// `Collection` describe e.g. "one autodelete timer per channel" without
/// the schema itself needing to change as channels are added or removed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConfigNode {
    /// An inline group of fields, rendered as a fieldset or card. If
    /// `toggle` is set, `children` are shown only while it is enabled --
    /// see `Toggled<T>` in docs/plans/gui-configuration.md section 5.4.
    Section {
        id: String,
        label: String,
        description: Option<String>,
        toggle: Option<Field>,
        children: Vec<ConfigNode>,
    },
    /// Zero or more keyed instances of `item`, e.g. one autodelete timer per
    /// channel. Which keys currently exist is data, carried in
    /// `ConfigValues`, not part of the schema.
    Collection {
        id: String,
        label: String,
        key: KeyKind,
        add_label: String,
        item: Vec<ConfigNode>,
    },
    Field(Field),
    /// Static help text or a warning, with no associated value.
    Note {
        markdown: String,
    },
}

impl ConfigNode {
    /// The id of this node, used as its path segment. `Note` has none,
    /// since it carries no value.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Section { id, .. } | Self::Collection { id, .. } => Some(id),
            Self::Field(field) => Some(&field.id),
            Self::Note { .. } => None,
        }
    }
}

/// A provider's full configuration schema: its identity plus its tree of
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigSchema {
    pub descriptor: ProviderDescriptor,
    pub root: Vec<ConfigNode>,
}

#[cfg(test)]
mod tests {
    use super::{ConfigNode, ConfigSchema};
    use crate::{
        descriptor::ProviderDescriptor,
        field::{Field, FieldKind, KeyKind},
        scope::ScopeKind,
    };

    #[test]
    fn test_node_id_for_each_variant() {
        let section = ConfigNode::Section {
            id: "logging".to_owned(),
            label: "server logging".to_owned(),
            description: None,
            toggle: None,
            children: vec![],
        };
        assert_eq!(section.id(), Some("logging"));

        let collection = ConfigNode::Collection {
            id: "timers".to_owned(),
            label: "autodelete timers".to_owned(),
            key: KeyKind::DiscordChannel,
            add_label: "add a channel".to_owned(),
            item: vec![],
        };
        assert_eq!(collection.id(), Some("timers"));

        let field = ConfigNode::Field(Field::new("channel", "log channel", FieldKind::Bool));
        assert_eq!(field.id(), Some("channel"));

        let note = ConfigNode::Note {
            markdown: "heads up!".to_owned(),
        };
        assert_eq!(note.id(), None);
    }

    #[test]
    fn test_config_schema_roundtrips_through_json() {
        let schema = ConfigSchema {
            descriptor: ProviderDescriptor {
                id: "logging".to_owned(),
                label: "server logging".to_owned(),
                description: None,
                icon: None,
                scopes: vec![ScopeKind::DiscordGuild],
            },
            root: vec![ConfigNode::Section {
                id: "logging".to_owned(),
                label: "server logging".to_owned(),
                description: None,
                toggle: Some(Field::new("enabled", "enable logging", FieldKind::Bool)),
                children: vec![ConfigNode::Field(Field::new(
                    "channel",
                    "log channel",
                    FieldKind::Ref {
                        kind: crate::field::RefKind::DiscordChannel,
                    },
                ))],
            }],
        };

        let json = serde_json::to_string(&schema).expect("serialize");
        let parsed: ConfigSchema = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, schema);
    }
}
