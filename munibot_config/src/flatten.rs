use std::collections::BTreeSet;

use serde_json::Value as Json;

use crate::{
    field::Field,
    node::ConfigNode,
    patch::FieldError,
    path::{ConfigPath, PathSegment},
    value::{ConfigValue, ConfigValues},
};

/// Walks `json` alongside `schema`, producing a flat `ConfigValues` map
/// keyed by path.
///
/// `json` is expected to be the `serde_json::Value` a `Configurable`
/// struct serializes to, so this never fails: the shape is already
/// guaranteed correct by that struct's own `Serialize` impl. Anything
/// unexpected (a missing key, the wrong json type) is treated as `Unset`
/// rather than an error.
pub fn flatten(schema: &[ConfigNode], json: &Json) -> ConfigValues {
    let mut out = ConfigValues::new();
    flatten_into(schema, json, &ConfigPath::root(), &mut out);
    out
}

fn flatten_into(nodes: &[ConfigNode], json: &Json, prefix: &ConfigPath, out: &mut ConfigValues) {
    for node in nodes {
        match node {
            ConfigNode::Field(field) => {
                let path = prefix.clone().field(&field.id);
                let value = json_field(json, &field.id);
                out.insert(path, config_value_from_json(&field.kind, value));
            }
            ConfigNode::Section {
                id,
                toggle,
                children,
                ..
            } => {
                let section_path = prefix.clone().field(id);
                let section_json = json_field(json, id);
                flatten_into(
                    &section_fields(toggle, children),
                    section_json,
                    &section_path,
                    out,
                );
            }
            ConfigNode::Collection { id, item, .. } => {
                let collection_path = prefix.clone().field(id);
                if let Some(Json::Object(items)) = json.get(id) {
                    for (key, item_json) in items {
                        let item_path = collection_path.clone().key(key.clone());
                        flatten_into(item, item_json, &item_path, out);
                    }
                }
            }
            ConfigNode::Note { .. } => {}
        }
    }
}

/// The inverse of `flatten`: rebuilds a `serde_json::Value` matching
/// `schema` from a flat `ConfigValues` map, so it can be deserialized back
/// into a `Configurable` struct.
///
/// Unlike `flatten`, this can fail: a `ConfigValue` supplied by a patch
/// might not be coercible to the field's real type (e.g.
/// `ConfigValue::Ref("not a number")` for a `ChannelRef` field). Every such
/// problem is collected, by path, rather than stopping at the first one.
pub fn unflatten(schema: &[ConfigNode], values: &ConfigValues) -> Result<Json, Vec<FieldError>> {
    let mut errors = Vec::new();
    let json = unflatten_into(schema, values, &ConfigPath::root(), &mut errors);
    if errors.is_empty() {
        Ok(json)
    } else {
        Err(errors)
    }
}

fn unflatten_into(
    nodes: &[ConfigNode],
    values: &ConfigValues,
    prefix: &ConfigPath,
    errors: &mut Vec<FieldError>,
) -> Json {
    let mut obj = serde_json::Map::new();

    for node in nodes {
        match node {
            ConfigNode::Field(field) => {
                let path = prefix.clone().field(&field.id);
                let value = values.get(&path).unwrap_or(&field.default);
                let json = match config_value_to_json(&field.kind, value) {
                    Ok(json) => json,
                    Err(message) => {
                        errors.push(FieldError::new(path, message));
                        Json::Null
                    }
                };
                obj.insert(field.id.clone(), json);
            }
            ConfigNode::Section {
                id,
                toggle,
                children,
                ..
            } => {
                let section_path = prefix.clone().field(id);
                let section_json = unflatten_into(
                    &section_fields(toggle, children),
                    values,
                    &section_path,
                    errors,
                );
                obj.insert(id.clone(), section_json);
            }
            ConfigNode::Collection { id, item, .. } => {
                let collection_path = prefix.clone().field(id);
                let mut items_obj = serde_json::Map::new();
                for key in collection_keys(values, &collection_path) {
                    let item_path = collection_path.clone().key(key.clone());
                    let item_json = unflatten_into(item, values, &item_path, errors);
                    items_obj.insert(key, item_json);
                }
                obj.insert(id.clone(), Json::Object(items_obj));
            }
            ConfigNode::Note { .. } => {}
        }
    }

    Json::Object(obj)
}

/// A `Section`'s toggle (if any) followed by its children, as one flat
/// list. Structurally, `Toggled<T>`'s `enabled` field and `T`'s own fields
/// share one json object (see `Toggled`'s `#[serde(flatten)]`), so walking
/// them as a single sibling list is what keeps flatten and unflatten
/// symmetric with that shape.
fn section_fields(toggle: &Option<Field>, children: &[ConfigNode]) -> Vec<ConfigNode> {
    toggle
        .iter()
        .cloned()
        .map(ConfigNode::Field)
        .chain(children.iter().cloned())
        .collect()
}

/// The json value at `key` within `json`, or `Json::Null` if `json` isn't
/// an object or has no such key.
fn json_field<'a>(json: &'a Json, key: &str) -> &'a Json {
    static NULL: Json = Json::Null;
    json.get(key).unwrap_or(&NULL)
}

/// The distinct collection item keys found in `values` under
/// `collection_path`, in sorted order.
fn collection_keys(values: &ConfigValues, collection_path: &ConfigPath) -> BTreeSet<String> {
    let prefix = collection_path.segments();
    values
        .keys()
        .filter_map(|path| {
            let segments = path.segments();
            if segments.len() > prefix.len() && segments[..prefix.len()] == *prefix {
                match &segments[prefix.len()] {
                    PathSegment::Key(key) => Some(key.clone()),
                    PathSegment::Field(_) => None,
                }
            } else {
                None
            }
        })
        .collect()
}

fn config_value_from_json(kind: &crate::field::FieldKind, json: &Json) -> ConfigValue {
    use crate::field::FieldKind;

    if json.is_null() {
        return ConfigValue::Unset;
    }

    match kind {
        FieldKind::Bool => json.as_bool().map_or(ConfigValue::Unset, ConfigValue::Bool),
        FieldKind::Int { .. } => json.as_i64().map_or(ConfigValue::Unset, ConfigValue::Int),
        FieldKind::Float { .. } => json.as_f64().map_or(ConfigValue::Unset, ConfigValue::Float),
        FieldKind::Text { .. } | FieldKind::Color | FieldKind::Url => json
            .as_str()
            .map_or(ConfigValue::Unset, |s| ConfigValue::Text(s.to_owned())),
        FieldKind::Duration { .. } => json
            .as_u64()
            .map_or(ConfigValue::Unset, ConfigValue::Duration),
        FieldKind::Choice { .. } => json
            .as_str()
            .map_or(ConfigValue::Unset, |s| ConfigValue::Choice(s.to_owned())),
        FieldKind::MultiChoice { .. } => json.as_array().map_or(ConfigValue::Unset, |items| {
            ConfigValue::MultiChoice(
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            )
        }),
        // ChannelRef and friends store a plain number in the struct (see
        // refs.rs); ConfigValue::Ref carries text instead, matching how
        // discord ids already travel over json elsewhere in munibot.
        FieldKind::Ref { .. } => json
            .as_u64()
            .map_or(ConfigValue::Unset, |id| ConfigValue::Ref(id.to_string())),
        FieldKind::List { item, .. } => json.as_array().map_or(ConfigValue::Unset, |items| {
            ConfigValue::List(
                items
                    .iter()
                    .map(|v| config_value_from_json(item, v))
                    .collect(),
            )
        }),
    }
}

fn config_value_to_json(
    kind: &crate::field::FieldKind,
    value: &ConfigValue,
) -> Result<Json, String> {
    use crate::field::FieldKind;

    match (kind, value) {
        (_, ConfigValue::Unset) => Ok(Json::Null),
        (FieldKind::Bool, ConfigValue::Bool(b)) => Ok(Json::Bool(*b)),
        (FieldKind::Int { .. }, ConfigValue::Int(i)) => Ok(Json::from(*i)),
        (FieldKind::Float { .. }, ConfigValue::Float(f)) => Ok(Json::from(*f)),
        (FieldKind::Text { .. } | FieldKind::Color | FieldKind::Url, ConfigValue::Text(s)) => {
            Ok(Json::String(s.clone()))
        }
        (FieldKind::Duration { .. }, ConfigValue::Duration(secs)) => Ok(Json::from(*secs)),
        (FieldKind::Choice { .. }, ConfigValue::Choice(id)) => Ok(Json::String(id.clone())),
        (FieldKind::MultiChoice { .. }, ConfigValue::MultiChoice(ids)) => Ok(Json::Array(
            ids.iter().map(|id| Json::String(id.clone())).collect(),
        )),
        (FieldKind::Ref { .. }, ConfigValue::Ref(s)) => s
            .parse::<u64>()
            .map(Json::from)
            .map_err(|_| format!("`{s}` is not a valid id")),
        (FieldKind::List { item, .. }, ConfigValue::List(items)) => {
            let mut json_items = Vec::with_capacity(items.len());
            for item_value in items {
                json_items.push(config_value_to_json(item, item_value)?);
            }
            Ok(Json::Array(json_items))
        }
        (kind, value) => Err(format!(
            "expected a value matching {kind:?}, found {value:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{flatten, unflatten};
    use crate::{
        field::{Field, FieldKind, KeyKind, RefKind},
        node::ConfigNode,
        path::ConfigPath,
        value::ConfigValue,
    };

    fn logging_schema() -> Vec<ConfigNode> {
        vec![ConfigNode::Section {
            id: "logging".to_owned(),
            label: "server logging".to_owned(),
            description: None,
            toggle: Some(Field::new("enabled", "enable logging", FieldKind::Bool)),
            children: vec![ConfigNode::Field(Field::new(
                "channel",
                "log channel",
                FieldKind::Ref {
                    kind: RefKind::DiscordChannel,
                },
            ))],
        }]
    }

    fn autodelete_schema() -> Vec<ConfigNode> {
        vec![ConfigNode::Collection {
            id: "timers".to_owned(),
            label: "autodelete timers".to_owned(),
            key: KeyKind::DiscordChannel,
            add_label: "add a channel".to_owned(),
            item: vec![
                ConfigNode::Field(Field::new(
                    "lifetime",
                    "message lifetime",
                    FieldKind::Duration {
                        min: None,
                        max: None,
                    },
                )),
                ConfigNode::Field(Field::new("mode", "cleanup mode", FieldKind::Choice {
                    options: vec![],
                })),
            ],
        }]
    }

    #[test]
    fn test_flatten_a_toggled_section() {
        let json = json!({ "logging": { "enabled": true, "channel": 6437 } });
        let values = flatten(&logging_schema(), &json);

        assert_eq!(
            values.get(&ConfigPath::root().field("logging").field("enabled")),
            Some(&ConfigValue::Bool(true))
        );
        assert_eq!(
            values.get(&ConfigPath::root().field("logging").field("channel")),
            Some(&ConfigValue::Ref("6437".to_owned()))
        );
    }

    #[test]
    fn test_flatten_missing_section_yields_unset_children() {
        let values = flatten(&logging_schema(), &json!({}));
        assert_eq!(
            values.get(&ConfigPath::root().field("logging").field("enabled")),
            Some(&ConfigValue::Unset)
        );
    }

    #[test]
    fn test_flatten_a_collection_with_multiple_items() {
        let json = json!({
            "timers": {
                "6437": { "lifetime": 3600, "mode": "after_silence" },
                "9001": { "lifetime": 60, "mode": "always" },
            }
        });
        let values = flatten(&autodelete_schema(), &json);

        assert_eq!(
            values.get(
                &ConfigPath::root()
                    .field("timers")
                    .key("6437")
                    .field("lifetime")
            ),
            Some(&ConfigValue::Duration(3600))
        );
        assert_eq!(
            values.get(&ConfigPath::root().field("timers").key("9001").field("mode")),
            Some(&ConfigValue::Choice("always".to_owned()))
        );
    }

    #[test]
    fn test_flatten_an_empty_collection() {
        let values = flatten(&autodelete_schema(), &json!({ "timers": {} }));
        assert!(values.is_empty());
    }

    #[test]
    fn test_unflatten_a_toggled_section() {
        let mut values = crate::value::ConfigValues::new();
        values.insert(
            ConfigPath::root().field("logging").field("enabled"),
            ConfigValue::Bool(true),
        );
        values.insert(
            ConfigPath::root().field("logging").field("channel"),
            ConfigValue::Ref("6437".to_owned()),
        );

        let json = unflatten(&logging_schema(), &values).expect("should not error");
        assert_eq!(
            json,
            json!({ "logging": { "enabled": true, "channel": 6437 } })
        );
    }

    #[test]
    fn test_unflatten_falls_back_to_field_defaults_for_missing_paths() {
        let json = unflatten(&logging_schema(), &crate::value::ConfigValues::new())
            .expect("should not error");
        // `enabled`'s default is ConfigValue::Unset -> null, and there's no
        // FieldKind coercion needed for that.
        assert_eq!(
            json,
            json!({ "logging": { "enabled": null, "channel": null } })
        );
    }

    #[test]
    fn test_unflatten_a_collection_with_multiple_items() {
        let mut values = crate::value::ConfigValues::new();
        values.insert(
            ConfigPath::root()
                .field("timers")
                .key("6437")
                .field("lifetime"),
            ConfigValue::Duration(3600),
        );
        values.insert(
            ConfigPath::root().field("timers").key("6437").field("mode"),
            ConfigValue::Choice("after_silence".to_owned()),
        );

        let json = unflatten(&autodelete_schema(), &values).expect("should not error");
        assert_eq!(
            json,
            json!({ "timers": { "6437": { "lifetime": 3600, "mode": "after_silence" } } })
        );
    }

    #[test]
    fn test_unflatten_reports_an_unparsable_ref_as_a_field_error() {
        let mut values = crate::value::ConfigValues::new();
        values.insert(
            ConfigPath::root().field("logging").field("channel"),
            ConfigValue::Ref("not a number".to_owned()),
        );

        let errors = unflatten(&logging_schema(), &values).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].path,
            ConfigPath::root().field("logging").field("channel")
        );
    }

    #[test]
    fn test_unflatten_collects_every_error_rather_than_stopping_at_the_first() {
        let mut values = crate::value::ConfigValues::new();
        values.insert(
            ConfigPath::root()
                .field("timers")
                .key("6437")
                .field("lifetime"),
            ConfigValue::Text("not a duration".to_owned()),
        );
        values.insert(
            ConfigPath::root().field("timers").key("9001").field("mode"),
            ConfigValue::Int(1),
        );

        let errors = unflatten(&autodelete_schema(), &values).unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_flatten_then_unflatten_roundtrips_for_logging() {
        let original = json!({ "logging": { "enabled": true, "channel": 6437 } });
        let values = flatten(&logging_schema(), &original);
        let rebuilt = unflatten(&logging_schema(), &values).expect("should not error");
        assert_eq!(rebuilt, original);
    }

    #[test]
    fn test_flatten_then_unflatten_roundtrips_for_autodelete_collections() {
        let original = json!({
            "timers": {
                "6437": { "lifetime": 3600, "mode": "after_silence" },
                "9001": { "lifetime": 60, "mode": "always" },
            }
        });
        let values = flatten(&autodelete_schema(), &original);
        let rebuilt = unflatten(&autodelete_schema(), &values).expect("should not error");
        assert_eq!(rebuilt, original);
    }
}
