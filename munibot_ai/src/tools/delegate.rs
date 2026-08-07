use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    persona::PersonaId,
    tools::{RiskTier, Tool, ToolCtx, ToolOutcome, delegator::DelegatorCell},
    types::{AiError, ToolSchema},
};

/// One persona the `delegate` tool may bring in - just enough to build the
/// tool's own input schema (so the model can only ever name a real,
/// delegable persona) and to check a requested id against at invocation
/// time.
#[derive(Clone, Debug, PartialEq)]
pub struct DelegablePersona {
    pub id: PersonaId,
    pub description: String,
}

#[derive(Deserialize)]
struct DelegateArgs {
    persona: String,
    task: String,
}

/// Brings a specialist persona in to handle one task, returning their final
/// answer as the tool result.
///
/// Tier [`RiskTier::Safe`]: delegation grants no new authority of its own -
/// the nested turn inherits the invoker's own `granted_tier` unchanged (see
/// [`ToolCtx::delegation_depth`]'s own doc comment), so a companion limited
/// to `NetworkRead` cannot reach a sandboxed builder persona by proxy just
/// by asking one to delegate on its behalf.
///
/// [`Tool::is_serial`] is `true`, so several delegate calls batched into one
/// iteration run one at a time rather than concurrently - each sees an
/// accurate [`ToolCtx::remaining_budget`] rather than racing sibling calls
/// for the same allowance.
pub struct DelegateTool {
    delegator: Arc<DelegatorCell>,
    personas: Vec<DelegablePersona>,
    max_depth: usize,
}

impl DelegateTool {
    pub fn new(
        delegator: Arc<DelegatorCell>,
        personas: Vec<DelegablePersona>,
        max_depth: usize,
    ) -> Self {
        Self {
            delegator,
            personas,
            max_depth,
        }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> &str {
        "Brings in a specialist to handle a task, then reports their answer back in your own \
         words. Always say out loud that you brought someone in - never present their work as your \
         own. Write a task that stands on its own: the specialist never sees this conversation, \
         only what you write here."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn is_serial(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        let ids: Vec<Value> = self
            .personas
            .iter()
            .map(|persona| Value::String(persona.id.0.clone()))
            .collect();
        let roster = self
            .personas
            .iter()
            .map(|persona| format!("- {}: {}", persona.id, persona.description))
            .collect::<Vec<_>>()
            .join("\n");

        ToolSchema::new(
            self.name(),
            self.description(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "persona": {
                        "type": "string",
                        "enum": ids,
                        "description": format!("Which specialist to bring in:\n{roster}"),
                    },
                    "task": {
                        "type": "string",
                        "description": "A self-contained brief for the specialist - they never \
                                         see this conversation, only this text.",
                    },
                },
                "required": ["persona", "task"],
            }),
        )
        .input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: DelegateArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        let Some(persona) = self.personas.iter().find(|p| p.id.0 == args.persona) else {
            let available = self
                .personas
                .iter()
                .map(|p| p.id.0.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return ToolOutcome::err(format!(
                "{:?} isn't a persona you can delegate to :< available: {available}",
                args.persona
            ));
        };

        let next_depth = ctx.delegation_depth + 1;
        if next_depth > self.max_depth {
            return ToolOutcome::err(format!(
                "delegating here would go {next_depth} levels deep, past the configured maximum \
                 of {} :< try answering this one yourself",
                self.max_depth
            ));
        }

        let Some(delegator) = self.delegator.get() else {
            return ToolOutcome::err("delegation isn't available right now :<".to_string());
        };

        let nested_ctx = ToolCtx {
            delegation_depth: next_depth,
            ..ctx.clone()
        };

        match delegator
            .delegate(&persona.id, args.task, &nested_ctx)
            .await
        {
            Ok(text) => ToolOutcome::ok(text),
            // a cancelled turn cannot be fixed by the model adjusting its call, the same
            // reasoning ToolOutcome::Fatal's own doc comment gives for cancellation generally
            Err(AiError::Cancelled) => ToolOutcome::fatal(AiError::Cancelled),
            Err(error) => ToolOutcome::err(format!("the specialist ran into a problem :< {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::tools::{ConversationId, Platform, delegator::Delegator};

    fn researcher() -> DelegablePersona {
        DelegablePersona {
            id: PersonaId::new("researcher"),
            description: "multi-step research with citations".to_string(),
        }
    }

    fn ctx(delegation_depth: usize) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Web,
            granted_tier: RiskTier::Safe,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth,
            remaining_budget: crate::harness::Budget::default(),
        }
    }

    /// Records every call it receives and returns one canned result, so a
    /// test can assert on exactly what the tool passed through - never a
    /// real turn, no provider, no network.
    ///
    /// `result` is consumed on the first call (each test in this module
    /// calls `delegate` at most once), avoiding needing `AiError: Clone`
    /// just to hand the same canned failure back on a second call nothing
    /// here ever makes.
    struct FakeDelegator {
        result: Mutex<Option<Result<String, AiError>>>,
        calls: Mutex<Vec<(PersonaId, String, usize)>>,
    }

    impl FakeDelegator {
        fn ok(text: &str) -> Self {
            Self::new(Ok(text.to_string()))
        }

        fn new(result: Result<String, AiError>) -> Self {
            Self {
                result: Mutex::new(Some(result)),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Delegator for FakeDelegator {
        async fn delegate(
            &self,
            persona: &PersonaId,
            task: String,
            ctx: &ToolCtx,
        ) -> Result<String, AiError> {
            self.calls
                .lock()
                .unwrap()
                .push((persona.clone(), task, ctx.delegation_depth));
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("delegate called more than once in this test")
        }
    }

    /// Wires `delegator` into a fresh cell and builds a tool over it,
    /// returning both so the caller keeps `delegator` alive for at least as
    /// long as the tool's own `Weak` needs it to be.
    fn tool_with(
        delegator: Arc<FakeDelegator>,
        max_depth: usize,
    ) -> (DelegateTool, Arc<FakeDelegator>) {
        let cell = Arc::new(DelegatorCell::new());
        cell.set(Arc::downgrade(&delegator) as std::sync::Weak<dyn Delegator>);
        (
            DelegateTool::new(cell, vec![researcher()], max_depth),
            delegator,
        )
    }

    #[test]
    fn test_tool_metadata() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("x")), 2);
        assert_eq!(tool.name(), "delegate");
        assert_eq!(tool.tier(), RiskTier::Safe);
        assert!(
            tool.is_serial(),
            "delegations must never race each other's budget"
        );
    }

    #[test]
    fn test_input_schema_enumerates_only_delegable_personas() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("x")), 2);
        let schema = tool.input_schema();
        let enum_values = schema["properties"]["persona"]["enum"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(enum_values, vec![json!("researcher")]);
    }

    #[tokio::test]
    async fn test_a_successful_delegation_returns_the_specialists_text() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("the answer is 42")), 2);
        let outcome = tool
            .invoke(
                json!({"persona": "researcher", "task": "what is the answer?"}),
                &ctx(0),
            )
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert_eq!(text, "the answer is 42"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_an_unknown_persona_is_a_recoverable_error() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("unused")), 2);
        let outcome = tool
            .invoke(
                json!({"persona": "wizard", "task": "cast a spell"}),
                &ctx(0),
            )
            .await;
        match outcome {
            ToolOutcome::Err(message) => assert!(message.contains("wizard")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("unused")), 2);
        let outcome = tool.invoke(json!({"persona": "researcher"}), &ctx(0)).await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_refuses_past_the_depth_cap() {
        // no delegate call should ever reach the fake - the depth check
        // must happen before it's ever consulted, so an unset result
        // wouldn't panic even if it did
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("unused")), 2);
        // already at depth 2, the configured maximum - one more would be depth 3
        let outcome = tool
            .invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(2))
            .await;
        match outcome {
            ToolOutcome::Err(message) => assert!(message.contains("levels deep")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_allows_exactly_up_to_the_depth_cap() {
        let (tool, _delegator) = tool_with(Arc::new(FakeDelegator::ok("ok")), 2);
        // at depth 1, delegating once more reaches exactly depth 2, the cap
        let outcome = tool
            .invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(1))
            .await;
        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_the_nested_context_carries_an_incremented_depth() {
        let (tool, delegator) = tool_with(Arc::new(FakeDelegator::ok("ok")), 2);

        tool.invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(1))
            .await;

        let calls = delegator.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].2, 2,
            "depth should be incremented for the nested turn"
        );
    }

    #[tokio::test]
    async fn test_the_task_is_passed_through_verbatim() {
        let (tool, delegator) = tool_with(Arc::new(FakeDelegator::ok("ok")), 2);

        tool.invoke(
            json!({"persona": "researcher", "task": "look into the history of tea"}),
            &ctx(0),
        )
        .await;

        let calls = delegator.calls.lock().unwrap();
        assert_eq!(calls[0].1, "look into the history of tea");
    }

    #[tokio::test]
    async fn test_a_cancelled_delegation_is_fatal_not_recoverable() {
        let (tool, _delegator) =
            tool_with(Arc::new(FakeDelegator::new(Err(AiError::Cancelled))), 2);

        let outcome = tool
            .invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(0))
            .await;
        assert!(
            matches!(outcome, ToolOutcome::Fatal(AiError::Cancelled)),
            "got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn test_a_non_cancellation_failure_is_recoverable() {
        let (tool, _delegator) = tool_with(
            Arc::new(FakeDelegator::new(Err(AiError::Other(
                "provider hiccup".to_string(),
            )))),
            2,
        );

        let outcome = tool
            .invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(0))
            .await;
        match outcome {
            ToolOutcome::Err(message) => assert!(message.contains("provider hiccup")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_delegator_wired_is_a_recoverable_error() {
        let cell = Arc::new(DelegatorCell::new());
        let tool = DelegateTool::new(cell, vec![researcher()], 2);

        let outcome = tool
            .invoke(json!({"persona": "researcher", "task": "a task"}), &ctx(0))
            .await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }
}
