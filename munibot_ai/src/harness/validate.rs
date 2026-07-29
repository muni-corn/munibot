use serde_json::Value;

/// Validates a tool call's arguments against its advertised JSON Schema,
/// returning a model-readable message describing the first violation found.
///
/// A model can only see this message, never a `jsonschema` error type, so the
/// message must stand on its own without any Rust-specific formatting leaking
/// through.
pub fn validate_tool_arguments(schema: &Value, arguments: &Value) -> Result<(), String> {
    jsonschema::validate(schema, arguments).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "num_results": {"type": "integer"}
            },
            "required": ["query"]
        })
    }

    #[test]
    fn test_valid_arguments_pass() {
        assert!(validate_tool_arguments(&schema(), &json!({"query": "cats"})).is_ok());
    }

    #[test]
    fn test_missing_required_field_fails() {
        let result = validate_tool_arguments(&schema(), &json!({"num_results": 5}));
        assert!(
            result.is_err(),
            "a call missing the required query field should be rejected"
        );
    }

    #[test]
    fn test_wrong_type_fails() {
        let result = validate_tool_arguments(&schema(), &json!({"query": 123}));
        assert!(
            result.is_err(),
            "a query that is a number instead of a string should be rejected"
        );
    }

    #[test]
    fn test_extra_unlisted_fields_are_allowed() {
        // JSON Schema permits additional properties by default; being permissive here
        // matches that default rather than silently becoming stricter than the
        // schema itself says
        let result = validate_tool_arguments(&schema(), &json!({"query": "cats", "extra": true}));
        assert!(
            result.is_ok(),
            "unlisted fields should be allowed unless the schema forbids them"
        );
    }

    #[test]
    fn test_error_message_is_readable() {
        let error =
            validate_tool_arguments(&schema(), &json!({})).expect_err("should fail validation");
        assert!(
            !error.is_empty(),
            "the model needs a non-empty explanation to correct itself"
        );
    }
}
