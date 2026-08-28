//! The host side of the wire protocol spoken with `munibot_toolagent` over
//! its unix socket.
//!
//! Hand-mirrored from `munibot_toolagent/src/protocol.rs`, deliberately not
//! shared through a common dependency - see `munibot_toolagent`'s own
//! `Cargo.toml` and `docs/plans/ai/milestone-4-sandbox.md`'s architecture
//! note for why. Keep the two copies in sync by hand if either one changes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One tool call sent from this host to a sandbox's tool agent.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolRequest {
    /// Correlates this call with its eventual [`ToolResponse`]. Assigned by
    /// [`crate::sandbox::RpcClient`]; the agent only ever echoes it back.
    pub id: u64,
    /// Which tool to run: `read`, `write`, `edit`, `bash`, `grep`, or `glob`.
    pub tool: String,
    /// The tool's own arguments, opaque to the dispatch layer.
    pub input: Value,
}

/// How one tool call finished, as reported by the agent.
///
/// Only ever `Ok` or `Err` - there is no `Fatal` variant here, unlike
/// [`crate::tools::ToolOutcome`] the host eventually converts this into. A
/// cancelled or unreachable agent is something [`crate::sandbox::RpcClient`]
/// itself detects (a dropped connection, a timeout), never something the
/// agent decides and reports back over a still-working connection.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResult {
    /// The tool succeeded; this text is the result shown to the model.
    Ok(String),
    /// The tool failed in a way the model can recover from by adjusting its
    /// next call.
    Err(String),
}

/// One tool call's result, sent from a sandbox's tool agent back to this
/// host.
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
        // this exact shape is what makes the two hand-mirrored copies of
        // this type (this one, and munibot_toolagent's) wire-compatible -
        // changing this without changing the other breaks the sandbox
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
            input: json!({"command": "ls -la", "timeout_secs": 30}),
        };

        let encoded = serde_json::to_vec(&request).expect("should serialize");
        let decoded: ToolRequest = serde_json::from_slice(&encoded).expect("should deserialize");

        assert_eq!(decoded, request);
    }
}
