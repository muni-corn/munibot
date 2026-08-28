//! The wire protocol spoken over the unix socket between the host's
//! `ai::sandbox` and this binary.
//!
//! Defined here and nowhere else in this crate's dependency graph, and
//! mirrored by hand on the host side in `munibot_ai::sandbox` rather than
//! shared through a common crate -- see this crate's `Cargo.toml` doc
//! comment and `docs/plans/ai/milestone-4-sandbox.md`'s architecture note for
//! why. Keep the two copies in sync by hand if either one changes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One tool call sent from the host to this agent.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolRequest {
    /// Correlates this call with its eventual [`ToolResponse`]. Assigned by
    /// the host; this agent only ever echoes it back.
    pub id: u64,
    /// Which tool to run: `read`, `write`, `edit`, `bash`, `grep`, or `glob`.
    pub tool: String,
    /// The tool's own arguments, opaque to the dispatch layer.
    pub input: Value,
}

/// How one tool call finished.
///
/// Only ever `Ok` or `Err` -- there is no `Fatal` variant here, unlike the
/// host-side `ToolOutcome` it eventually becomes. A cancelled or unreachable
/// agent is something the host's `RpcClient` itself detects (a dropped
/// connection, a timeout), never something this agent decides and reports
/// back over a still-working connection.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResult {
    /// The tool succeeded; this text is the result shown to the model.
    Ok(String),
    /// The tool failed in a way the model can recover from by adjusting its
    /// next call.
    Err(String),
}

/// One tool call's result, sent from this agent back to the host.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolResponse {
    /// Echoes the [`ToolRequest::id`] this response answers.
    pub id: u64,
    pub result: ToolResult,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_tool_request_round_trips_through_json() {
        let request = ToolRequest {
            id: 7,
            tool: "read".to_string(),
            input: json!({"path": "src/main.rs"}),
        };

        let encoded = serde_json::to_vec(&request).expect("should serialize");
        let decoded: ToolRequest = serde_json::from_slice(&encoded).expect("should deserialize");

        assert_eq!(decoded, request);
    }

    #[test]
    fn test_tool_response_ok_round_trips_through_json() {
        let response = ToolResponse {
            id: 7,
            result: ToolResult::Ok("file contents".to_string()),
        };

        let encoded = serde_json::to_vec(&response).expect("should serialize");
        let decoded: ToolResponse = serde_json::from_slice(&encoded).expect("should deserialize");

        assert_eq!(decoded, response);
    }

    #[test]
    fn test_tool_response_err_round_trips_through_json() {
        let response = ToolResponse {
            id: 7,
            result: ToolResult::Err("no such file :<".to_string()),
        };

        let encoded = serde_json::to_vec(&response).expect("should serialize");
        let decoded: ToolResponse = serde_json::from_slice(&encoded).expect("should deserialize");

        assert_eq!(decoded, response);
    }

    #[test]
    fn test_tool_result_serializes_as_a_tagged_snake_case_enum() {
        let encoded =
            serde_json::to_value(ToolResult::Ok("hi".to_string())).expect("should serialize");
        assert_eq!(encoded, json!({"ok": "hi"}));

        let encoded =
            serde_json::to_value(ToolResult::Err("oops".to_string())).expect("should serialize");
        assert_eq!(encoded, json!({"err": "oops"}));
    }

    #[test]
    fn test_tool_request_input_can_be_any_json_shape() {
        let request = ToolRequest {
            id: 1,
            tool: "bash".to_string(),
            input: json!({"command": "ls -la", "timeout_secs": 30, "nested": {"a": [1, 2, 3]}}),
        };

        let encoded = serde_json::to_vec(&request).expect("should serialize");
        let decoded: ToolRequest = serde_json::from_slice(&encoded).expect("should deserialize");

        assert_eq!(decoded, request);
    }
}
