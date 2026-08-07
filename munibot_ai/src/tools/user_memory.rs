use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    tools::{RiskTier, Tool, ToolCtx, ToolOutcome},
    types::{AiError, ToolSchema},
};

/// What the `remember` and `forget` tools need from long-term memory.
///
/// A narrow, tools-local mirror of [`crate::memory::MemoryStore`], rather
/// than a dependency on it directly: `ai::tools` sits below `ai::memory` in
/// this crate's dependency graph (`memory` already depends on `tools` for
/// [`crate::tools::ConversationId`]/[`crate::tools::Platform`]), so `tools`
/// referencing that module back would create a cycle. Whatever constructs
/// these tools bridges the two - see `crate::memory::MemoryToolBackend`.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError>;
    async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError>;
}

#[derive(Deserialize, JsonSchema)]
struct RememberArgs {
    /// A short, stable name for this fact, used to update or forget it
    /// later.
    key: String,
    /// What to remember.
    value: String,
}

/// Lets a persona remember a fact about the invoking user, across
/// conversations.
///
/// Refuses when the invoker has not opted into memory, telling the model to
/// point at the memory settings rather than pretending to have remembered
/// something it did not. There is deliberately no recall tool - retrieval is
/// the host's job (rendered into the system prompt), not the model's.
pub struct RememberTool {
    backend: Arc<dyn MemoryBackend>,
}

impl RememberTool {
    pub fn new(backend: Arc<dyn MemoryBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        "Remembers a fact about the person you're talking to, under a short key, so you can bring \
         it up in a future conversation. Only works if they have opted into memory."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<RememberArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: RememberArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        match self
            .backend
            .record(ctx.user_id, &args.key, &args.value)
            .await
        {
            Ok(()) => ToolOutcome::ok(format!("remembered {:?}", args.key)),
            Err(error) => ToolOutcome::err(format!(
                "couldn't remember that :< {error} - mention they can opt into memory in settings \
                 if they haven't"
            )),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct ForgetArgs {
    /// The key of a fact previously recorded with `remember`.
    key: String,
}

/// Lets a persona forget a specific fact it was previously told to remember.
pub struct ForgetTool {
    backend: Arc<dyn MemoryBackend>,
}

impl ForgetTool {
    pub fn new(backend: Arc<dyn MemoryBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for ForgetTool {
    fn name(&self) -> &str {
        "forget"
    }

    fn description(&self) -> &str {
        "Forgets a specific fact previously recorded with the remember tool, identified by its key."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<ForgetArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ForgetArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        match self.backend.forget(ctx.user_id, &args.key).await {
            Ok(()) => ToolOutcome::ok(format!("forgot {:?}", args.key)),
            Err(error) => ToolOutcome::err(format!("couldn't forget that :< {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::tools::Platform;

    fn ctx() -> ToolCtx {
        ToolCtx {
            user_id: 7,
            platform: Platform::Web,
            granted_tier: RiskTier::Safe,
            guild_id: None,
            conversation_id: crate::tools::ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
            delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    /// A [`MemoryBackend`] that either always succeeds (recording what it was
    /// called with) or always refuses, depending on construction.
    struct FakeBackend {
        refuse: bool,
        calls: Mutex<Vec<(String, String, String)>>,
    }

    impl FakeBackend {
        fn allowing() -> Self {
            Self {
                refuse: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn refusing() -> Self {
            Self {
                refuse: true,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl MemoryBackend for FakeBackend {
        async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError> {
            if self.refuse {
                return Err(AiError::Config("memory is off :<".to_string()));
            }
            self.calls.lock().unwrap().push((
                "record".to_string(),
                format!("{user_id}:{key}"),
                value.to_string(),
            ));
            Ok(())
        }

        async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError> {
            if self.refuse {
                return Err(AiError::Config("memory is off :<".to_string()));
            }
            self.calls.lock().unwrap().push((
                "forget".to_string(),
                format!("{user_id}:{key}"),
                String::new(),
            ));
            Ok(())
        }
    }

    #[test]
    fn test_remember_tool_metadata() {
        let tool = RememberTool::new(Arc::new(FakeBackend::allowing()));
        assert_eq!(tool.name(), "remember");
        assert_eq!(tool.tier(), RiskTier::Safe);
    }

    #[test]
    fn test_forget_tool_metadata() {
        let tool = ForgetTool::new(Arc::new(FakeBackend::allowing()));
        assert_eq!(tool.name(), "forget");
        assert_eq!(tool.tier(), RiskTier::Safe);
    }

    #[tokio::test]
    async fn test_remember_records_through_the_backend_using_the_invokers_user_id() {
        let backend = Arc::new(FakeBackend::allowing());
        let tool = RememberTool::new(backend.clone());

        let outcome = tool
            .invoke(json!({"key": "favorite_color", "value": "purple"}), &ctx())
            .await;

        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");
        assert_eq!(*backend.calls.lock().unwrap(), vec![(
            "record".to_string(),
            "7:favorite_color".to_string(),
            "purple".to_string()
        )]);
    }

    #[tokio::test]
    async fn test_remember_is_recoverable_when_the_backend_refuses() {
        let tool = RememberTool::new(Arc::new(FakeBackend::refusing()));
        let outcome = tool.invoke(json!({"key": "k", "value": "v"}), &ctx()).await;

        match outcome {
            ToolOutcome::Err(text) => assert!(
                text.contains("memory") || text.contains("opt"),
                "the model should learn why, got {text:?}"
            ),
            other => panic!("a backend refusal must be recoverable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_remember_malformed_arguments_are_recoverable() {
        let tool = RememberTool::new(Arc::new(FakeBackend::allowing()));
        let outcome = tool.invoke(json!({"key": "k"}), &ctx()).await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_forget_calls_the_backend_with_the_invokers_user_id() {
        let backend = Arc::new(FakeBackend::allowing());
        let tool = ForgetTool::new(backend.clone());

        let outcome = tool.invoke(json!({"key": "favorite_color"}), &ctx()).await;

        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");
        assert_eq!(*backend.calls.lock().unwrap(), vec![(
            "forget".to_string(),
            "7:favorite_color".to_string(),
            String::new()
        )]);
    }

    #[tokio::test]
    async fn test_forget_is_recoverable_when_the_backend_refuses() {
        let tool = ForgetTool::new(Arc::new(FakeBackend::refusing()));
        let outcome = tool.invoke(json!({"key": "k"}), &ctx()).await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_forget_malformed_arguments_are_recoverable() {
        let tool = ForgetTool::new(Arc::new(FakeBackend::allowing()));
        let outcome = tool.invoke(json!({}), &ctx()).await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[test]
    fn test_remember_input_schema_has_key_and_value() {
        let schema = RememberTool::new(Arc::new(FakeBackend::allowing())).input_schema();
        assert!(schema["properties"].get("key").is_some());
        assert!(schema["properties"].get("value").is_some());
    }

    #[test]
    fn test_forget_input_schema_has_key() {
        let schema = ForgetTool::new(Arc::new(FakeBackend::allowing())).input_schema();
        assert!(schema["properties"].get("key").is_some());
    }
}
