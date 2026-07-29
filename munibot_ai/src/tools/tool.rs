use async_trait::async_trait;
use serde_json::Value;

use crate::{
    tools::{RiskTier, ToolCtx},
    types::{AiError, ToolSchema},
};

/// The result of one tool invocation.
#[derive(Debug)]
pub enum ToolOutcome {
    /// The tool succeeded; this text is shown to the model as the tool result.
    Ok(String),
    /// The tool failed in a way the model can recover from by adjusting its
    /// next call: a bad argument, a resource that was not found, an
    /// authorization refusal. Shown to the model as an error tool result so
    /// it can correct itself, rather than aborting the turn.
    Err(String),
    /// The tool failed in a way that aborts the whole turn: a cancelled
    /// context, a provider outage reaching through the tool, anything the
    /// model retrying its own call cannot fix.
    Fatal(AiError),
}

impl ToolOutcome {
    /// Builds a success outcome.
    pub fn ok(text: impl Into<String>) -> Self {
        Self::Ok(text.into())
    }

    /// Builds a model-recoverable failure.
    pub fn err(text: impl Into<String>) -> Self {
        Self::Err(text.into())
    }

    /// Builds a turn-aborting failure.
    pub fn fatal(error: impl Into<AiError>) -> Self {
        Self::Fatal(error.into())
    }

    /// Returns `true` if this outcome succeeded.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// One capability a persona can reach for.
///
/// A tool's authority comes from the [`ToolCtx`] it is invoked with, never from
/// the arguments the model supplies - every implementation above
/// [`RiskTier::Safe`] must call [`ToolCtx::require_tier`] as its first action.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The name the model calls. Must be unique across the registry it is added
    /// to.
    fn name(&self) -> &str;

    /// When and why to use this tool, shown to the model verbatim. This is a
    /// prompt, not documentation - it is the only thing telling the model
    /// when to reach for this tool.
    fn description(&self) -> &str;

    /// This tool's own risk tier, fixed at registration. Neither a persona's
    /// [`crate::tools::ToolSelection`] nor an invoker's granted tier can
    /// lift a tool above where it was registered.
    fn tier(&self) -> RiskTier;

    /// A JSON Schema object describing this tool's arguments.
    fn input_schema(&self) -> Value;

    /// Runs the tool against one set of arguments.
    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome;

    /// Builds this tool's advertised schema, for handing to a provider.
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description(), self.input_schema())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct PingTool;

    #[async_trait]
    impl Tool for PingTool {
        fn name(&self) -> &str {
            "ping"
        }

        fn description(&self) -> &str {
            "Does nothing, successfully."
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }

        async fn invoke(&self, _input: Value, ctx: &ToolCtx) -> ToolOutcome {
            match ctx.require_tier(self.tier()) {
                Ok(()) => ToolOutcome::ok("pong"),
                Err(error) => ToolOutcome::fatal(error),
            }
        }
    }

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: crate::tools::Platform::Discord,
            granted_tier,
            guild_id: None,
            conversation_id: crate::tools::ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn test_invoke_returns_the_scripted_outcome() {
        let outcome = PingTool.invoke(json!({}), &ctx(RiskTier::Safe)).await;
        assert!(matches!(outcome, ToolOutcome::Ok(text) if text == "pong"));
    }

    #[test]
    fn test_schema_derives_from_the_trait_methods() {
        let schema = PingTool.schema();
        assert_eq!(schema.name, "ping");
        assert_eq!(schema.description, "Does nothing, successfully.");
        assert_eq!(
            schema.input_schema,
            json!({"type": "object", "properties": {}})
        );
    }

    #[test]
    fn test_outcome_constructors() {
        assert!(ToolOutcome::ok("done").is_ok());
        assert!(!ToolOutcome::err("nope").is_ok());
        assert!(!ToolOutcome::fatal(AiError::Cancelled).is_ok());
    }

    #[tokio::test]
    async fn test_tool_can_be_boxed_as_a_trait_object() {
        // this is the whole point of the trait: it must be object-safe so the registry
        // can hold Arc<dyn Tool> for tools of many concrete types
        let tool: std::sync::Arc<dyn Tool> = std::sync::Arc::new(PingTool);
        let outcome = tool.invoke(json!({}), &ctx(RiskTier::Safe)).await;
        assert!(outcome.is_ok());
    }
}
