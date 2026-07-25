use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A single step in a `ConfigPath`: either a named struct field or a
/// collection item's key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSegment {
    /// A named struct field, e.g. `channel` in `logging.channel`.
    Field(String),
    /// A collection item's key, e.g. `6437...` in `timers[6437...]`.
    Key(String),
}

/// A location within a provider's configuration tree.
///
/// Used both to address a field in a `ConfigSchema` and as the key of a
/// `ConfigValues` map, so a provider's entire configuration -- however
/// deeply nested -- can travel as one flat map rather than a mirrored
/// nested object. See docs/plans/gui-configuration.md section 5.2.
///
/// Displays and parses as e.g. `logging.channel` or
/// `timers[6437...].lifetime`, which is also this type's wire (json)
/// representation via `Serialize`/`Deserialize`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ConfigPath(Vec<PathSegment>);

impl ConfigPath {
    /// An empty path, referring to the root of a schema.
    pub fn root() -> Self {
        Self(Vec::new())
    }

    /// Appends a field segment, returning the extended path.
    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.0.push(PathSegment::Field(name.into()));
        self
    }

    /// Appends a collection key segment, returning the extended path.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.0.push(PathSegment::Key(key.into()));
        self
    }

    /// This path's segments, in order from the root.
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// True if this path has no segments, i.e. it refers to the schema's
    /// root rather than any particular field.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

/// An error parsing a `ConfigPath` from its string representation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigPathError {
    #[error("empty key `[]` in path: {0}")]
    EmptyKey(String),

    #[error("empty field name in path: {0}")]
    EmptyField(String),

    #[error("unterminated `[` in path: {0}")]
    UnterminatedKey(String),
}

impl fmt::Display for ConfigPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Field(name) => {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    write!(f, "{name}")?;
                }
                PathSegment::Key(key) => write!(f, "[{key}]")?,
            }
        }
        Ok(())
    }
}

impl FromStr for ConfigPath {
    type Err = ConfigPathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('.') {
            return Err(ConfigPathError::EmptyField(s.to_owned()));
        }

        let mut segments = Vec::new();
        let mut rest = s;

        while !rest.is_empty() {
            if let Some(after_dot) = rest.strip_prefix('.') {
                // a dot must be followed by a real field name -- not
                // another separator or the end of the string -- otherwise
                // it's an empty segment like the one in "logging..channel".
                if after_dot.is_empty() || after_dot.starts_with(['.', '[']) {
                    return Err(ConfigPathError::EmptyField(s.to_owned()));
                }
                rest = after_dot;
                continue;
            }

            if let Some(after_bracket) = rest.strip_prefix('[') {
                let Some(end) = after_bracket.find(']') else {
                    return Err(ConfigPathError::UnterminatedKey(s.to_owned()));
                };
                let key = &after_bracket[..end];
                if key.is_empty() {
                    return Err(ConfigPathError::EmptyKey(s.to_owned()));
                }
                segments.push(PathSegment::Key(key.to_owned()));
                rest = &after_bracket[end + 1..];
                continue;
            }

            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let field = &rest[..end];
            if field.is_empty() {
                return Err(ConfigPathError::EmptyField(s.to_owned()));
            }
            segments.push(PathSegment::Field(field.to_owned()));
            rest = &rest[end..];
        }

        Ok(Self(segments))
    }
}

// serialized as a plain string rather than deriving on the segment enum, so
// a ConfigPath is a normal json string both on the wire and as a
// serde_json object key (BTreeMap<ConfigPath, _> needs a string-like key to
// serialize as a json object at all).
impl Serialize for ConfigPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ConfigPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigPath, ConfigPathError, PathSegment};

    #[test]
    fn test_root_path_displays_as_empty_string() {
        assert_eq!(ConfigPath::root().to_string(), "");
        assert!(ConfigPath::root().is_root());
    }

    #[test]
    fn test_single_field_path_roundtrips() {
        let path = ConfigPath::root().field("channel");
        assert_eq!(path.to_string(), "channel");
        assert_eq!(path.to_string().parse::<ConfigPath>().unwrap(), path);
    }

    #[test]
    fn test_nested_field_path_roundtrips() {
        let path = ConfigPath::root().field("logging").field("channel");
        assert_eq!(path.to_string(), "logging.channel");
        assert_eq!(path.to_string().parse::<ConfigPath>().unwrap(), path);
    }

    #[test]
    fn test_collection_item_path_roundtrips() {
        let path = ConfigPath::root()
            .field("timers")
            .key("6437")
            .field("lifetime");
        assert_eq!(path.to_string(), "timers[6437].lifetime");
        assert_eq!(path.to_string().parse::<ConfigPath>().unwrap(), path);
    }

    #[test]
    fn test_bare_collection_item_path_has_no_leading_dot() {
        // `timers[6437]`, not `timers.[6437]` -- brackets don't need a dot
        // separator since they're unambiguous on their own.
        let path = ConfigPath::root().field("timers").key("6437");
        assert_eq!(path.to_string(), "timers[6437]");
    }

    #[test]
    fn test_parse_rejects_empty_key() {
        assert_eq!(
            "timers[]".parse::<ConfigPath>(),
            Err(ConfigPathError::EmptyKey("timers[]".to_owned()))
        );
    }

    #[test]
    fn test_parse_rejects_unterminated_key() {
        assert_eq!(
            "timers[6437".parse::<ConfigPath>(),
            Err(ConfigPathError::UnterminatedKey("timers[6437".to_owned()))
        );
    }

    #[test]
    fn test_parse_rejects_empty_field_from_leading_dot_pair() {
        assert_eq!(
            "logging..channel".parse::<ConfigPath>(),
            Err(ConfigPathError::EmptyField("logging..channel".to_owned()))
        );
    }

    #[test]
    fn test_segments_reflects_construction_order() {
        let path = ConfigPath::root().field("a").key("b").field("c");
        assert_eq!(path.segments(), &[
            PathSegment::Field("a".to_owned()),
            PathSegment::Key("b".to_owned()),
            PathSegment::Field("c".to_owned()),
        ]);
    }

    #[test]
    fn test_config_path_serializes_as_a_plain_json_string() {
        let path = ConfigPath::root().field("timers").key("6437");
        let json = serde_json::to_string(&path).expect("serialize");
        assert_eq!(json, "\"timers[6437]\"");
    }

    #[test]
    fn test_config_path_deserializes_from_a_plain_json_string() {
        let path: ConfigPath =
            serde_json::from_str("\"timers[6437].lifetime\"").expect("deserialize");
        assert_eq!(
            path,
            ConfigPath::root()
                .field("timers")
                .key("6437")
                .field("lifetime")
        );
    }

    #[test]
    fn test_config_path_as_a_btreemap_key_serializes_to_a_json_object() {
        use std::collections::BTreeMap;

        let mut map = BTreeMap::new();
        map.insert(ConfigPath::root().field("channel"), 1);
        let json = serde_json::to_string(&map).expect("serialize");
        assert_eq!(json, "{\"channel\":1}");
    }
}
