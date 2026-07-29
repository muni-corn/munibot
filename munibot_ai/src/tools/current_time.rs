use async_trait::async_trait;
use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    tools::{RiskTier, Tool, ToolCtx, ToolOutcome},
    types::ToolSchema,
};

#[derive(Deserialize, JsonSchema)]
struct CurrentTimeArgs {
    /// An IANA timezone name, such as `America/New_York` or `Europe/London`.
    /// Defaults to UTC when omitted.
    timezone: Option<String>,
}

/// Tells the model the current date and time.
///
/// Trivial, but proves the whole tool path end to end, and is genuinely useful
/// on its own: a model has no clock of its own and cannot know the current date
/// without being told.
pub struct CurrentTimeTool;

#[async_trait]
impl Tool for CurrentTimeTool {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Returns the current date and time, optionally in a specific IANA timezone (for example \
         `America/New_York` or `Europe/London`). Defaults to UTC when no timezone is given."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<CurrentTimeArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let args: CurrentTimeArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        match args.timezone {
            None => ToolOutcome::ok(Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()),
            Some(name) => match name.parse::<chrono_tz::Tz>() {
                Ok(zone) => {
                    let now = Utc::now().with_timezone(&zone);
                    ToolOutcome::ok(now.format("%Y-%m-%d %H:%M:%S %Z").to_string())
                }
                Err(_) => ToolOutcome::err(format!(
                    "{name:?} isn't a recognized IANA timezone name (try `America/New_York` or \
                     `Europe/London`) :<"
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tools::{ConversationId, Platform};

    fn ctx() -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier: RiskTier::Safe,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn test_tool_metadata() {
        let tool = CurrentTimeTool;
        assert_eq!(tool.name(), "current_time");
        assert_eq!(tool.tier(), RiskTier::Safe);
    }

    #[test]
    fn test_input_schema_derives_a_timezone_property() {
        let schema = CurrentTimeTool.input_schema();
        assert!(
            schema["properties"].get("timezone").is_some(),
            "the schema should expose an optional timezone property"
        );
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(
            !required.contains(&json!("timezone")),
            "timezone is optional and must not be required"
        );
    }

    #[tokio::test]
    async fn test_no_timezone_defaults_to_utc() {
        let outcome = CurrentTimeTool.invoke(json!({}), &ctx()).await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.ends_with("UTC"), "got {text:?}"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_valid_timezone_is_honored() {
        let outcome = CurrentTimeTool
            .invoke(json!({"timezone": "America/New_York"}), &ctx())
            .await;
        assert!(
            matches!(outcome, ToolOutcome::Ok(_)),
            "a real IANA name should succeed: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn test_invalid_timezone_is_a_recoverable_error() {
        let outcome = CurrentTimeTool
            .invoke(json!({"timezone": "Mars/Olympus_Mons"}), &ctx())
            .await;
        match outcome {
            ToolOutcome::Err(message) => {
                assert!(message.contains("recognized"), "got {message:?}");
            }
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        // timezone must be a string, not a number
        let outcome = CurrentTimeTool.invoke(json!({"timezone": 5}), &ctx()).await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }
}
