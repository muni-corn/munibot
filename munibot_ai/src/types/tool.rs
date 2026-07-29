use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a tool is called, what it does, and what it accepts.
///
/// This is the description handed to the model. The executable side of a tool
/// lives in `munibot_ai_tools`; this type is only the data a provider needs to
/// advertise it.
///
/// The description is not documentation — it is a prompt. It is the only thing
/// telling the model when to reach for this tool, so it deserves the same care
/// as a system prompt.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolSchema {
    /// The name the model calls. Must match the registered tool name exactly.
    pub name: String,
    /// When and why to use this tool, written for the model.
    pub description: String,
    /// A JSON Schema object describing the arguments.
    pub input_schema: Value,
}

impl ToolSchema {
    /// Builds a schema from an already-constructed JSON Schema value.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: strip_meta_keywords(input_schema),
        }
    }

    /// Derives a schema from a Rust type's `JsonSchema` implementation.
    ///
    /// This is how tools should normally declare their arguments, so that the
    /// schema and the struct the tool deserializes into cannot drift apart.
    ///
    /// # Example
    /// ```
    /// use munibot_ai::ToolSchema;
    /// use schemars::JsonSchema;
    ///
    /// #[derive(JsonSchema)]
    /// struct CurrentTimeArgs {
    ///     /// An IANA timezone name.
    ///     timezone: Option<String>,
    /// }
    ///
    /// let schema =
    ///     ToolSchema::from_schemars::<CurrentTimeArgs>("current_time", "Get the current time.");
    /// assert_eq!(schema.input_schema["type"], "object");
    /// ```
    pub fn from_schemars<T: JsonSchema>(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self::new(name, description, schemars::schema_for!(T).to_value())
    }

    /// Builds a schema for a tool that takes no arguments.
    pub fn no_arguments(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(
            name,
            description,
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }
}

/// Removes JSON Schema metadata that providers do not want.
///
/// `$schema` and `title` are noise to every provider we support, and some
/// reject unknown top-level keywords outright.
///
/// Note that `$defs` and `$ref` are deliberately left alone. Providers vary in
/// how well they handle references, so if a nested type ever misbehaves the fix
/// is to inline the definitions here rather than to work around it at the call
/// site.
fn strip_meta_keywords(mut schema: Value) -> Value {
    if let Value::Object(map) = &mut schema {
        map.remove("$schema");
        map.remove("title");
    }
    schema
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct SearchArgs {
        /// What to search for.
        query: String,
        /// How many results to return.
        num_results: Option<u32>,
    }

    #[test]
    fn test_from_schemars_produces_an_object_schema() {
        let schema = ToolSchema::from_schemars::<SearchArgs>("web_search", "Search the web.");
        assert_eq!(
            schema.input_schema["type"], "object",
            "a derived argument schema should be an object"
        );
    }

    #[test]
    fn test_from_schemars_keeps_field_names_and_docs() {
        let schema = ToolSchema::from_schemars::<SearchArgs>("web_search", "Search the web.");
        let properties = &schema.input_schema["properties"];

        assert!(
            properties.get("query").is_some(),
            "query should be a property"
        );
        assert!(
            properties.get("num_results").is_some(),
            "num_results should be a property"
        );
        assert_eq!(
            properties["query"]["description"], "What to search for.",
            "doc comments should become descriptions the model can read"
        );
    }

    #[test]
    fn test_from_schemars_marks_required_fields() {
        let schema = ToolSchema::from_schemars::<SearchArgs>("web_search", "Search the web.");
        let required = schema.input_schema["required"]
            .as_array()
            .expect("required should be an array")
            .clone();

        assert!(
            required.contains(&json!("query")),
            "a non-optional field should be required"
        );
        assert!(
            !required.contains(&json!("num_results")),
            "an Option field should not be required"
        );
    }

    #[test]
    fn test_meta_keywords_are_stripped() {
        let schema = ToolSchema::from_schemars::<SearchArgs>("web_search", "Search the web.");
        assert!(
            schema.input_schema.get("$schema").is_none(),
            "$schema is noise to every provider and some reject it"
        );
        assert!(
            schema.input_schema.get("title").is_none(),
            "title is noise to every provider"
        );
    }

    #[test]
    fn test_new_also_strips_meta_keywords() {
        let schema = ToolSchema::new(
            "t",
            "d",
            json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object"}),
        );
        assert!(
            schema.input_schema.get("$schema").is_none(),
            "hand-written schemas should be cleaned too"
        );
    }

    #[test]
    fn test_no_arguments_builds_an_empty_object() {
        let schema = ToolSchema::no_arguments("ping", "Do nothing.");
        assert_eq!(
            schema.input_schema,
            json!({"type": "object", "properties": {}}),
            "a no-argument tool still needs an object schema"
        );
    }

    #[test]
    fn test_schema_roundtrips() {
        let schema = ToolSchema::from_schemars::<SearchArgs>("web_search", "Search the web.");
        let encoded = serde_json::to_string(&schema).expect("should serialize");
        let decoded: ToolSchema = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, schema, "tool schema should survive a roundtrip");
    }
}
