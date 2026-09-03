use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    tools::{ConversationId, RiskTier, Tool, ToolCtx, ToolOutcome},
    types::ToolSchema,
};

/// How far along one task is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// One task in a todo list.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct TodoItem {
    /// A short description of the task.
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Deserialize, JsonSchema)]
struct TodoWriteArgs {
    /// The complete, current list of tasks. Replaces whatever list was
    /// previously recorded for this conversation - it does not append to
    /// it.
    todos: Vec<TodoItem>,
}

/// Which invocation a todo list belongs to: a chat conversation or a
/// pipeline run, never a fabricated identifier standing in for whichever
/// one an invocation lacks - see `crate::tools::ToolCtx`'s own doc
/// comment on why `conversation_id` and `pipeline_id` are never both
/// `Some` at once.
type TodoScopeKey = (Option<ConversationId>, Option<i64>);

/// A scratchpad for tracking progress through a long research or pipeline run.
///
/// Stored per conversation or per pipeline run, in memory only for now - a
/// persona working through many steps can keep its plan visible rather than
/// losing track partway through. Each call replaces the whole list rather
/// than appending to it, matching how the model is expected to resend its
/// complete current plan every time it updates.
#[derive(Default)]
pub struct TodoWriteTool {
    lists: Mutex<HashMap<TodoScopeKey, Vec<TodoItem>>>,
}

impl TodoWriteTool {
    /// Builds an empty scratchpad.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current todo list for a conversation or pipeline run, for a
    /// platform adapter to render alongside the tool's own `ToolFinished`
    /// event.
    pub fn current(
        &self,
        conversation_id: Option<ConversationId>,
        pipeline_id: Option<i64>,
    ) -> Vec<TodoItem> {
        self.lists
            .lock()
            .unwrap()
            .get(&(conversation_id, pipeline_id))
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo_write"
    }

    fn description(&self) -> &str {
        "Records or updates your current task list for this conversation, so progress through a \
         multi-step task stays visible. Always send the complete, current list of tasks - this \
         replaces whatever was recorded before, it does not append to it."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Safe
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<TodoWriteArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: TodoWriteArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        let count = args.todos.len();
        self.lists
            .lock()
            .unwrap()
            .insert((ctx.conversation_id, ctx.pipeline_id), args.todos);

        ToolOutcome::ok(format!("recorded {count} todo item(s)"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tools::Platform;

    fn ctx(conversation_id: ConversationId) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier: RiskTier::Safe,
            guild_id: None,
            conversation_id: Some(conversation_id),
            pipeline_id: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
            delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    fn pipeline_ctx(pipeline_id: i64) -> ToolCtx {
        ToolCtx {
            conversation_id: None,
            pipeline_id: Some(pipeline_id),
            ..ctx(ConversationId(0))
        }
    }

    fn todos(pairs: &[(&str, TodoStatus)]) -> Value {
        json!({
            "todos": pairs.iter().map(|(content, status)| json!({"content": content, "status": status})).collect::<Vec<_>>()
        })
    }

    #[test]
    fn test_tool_metadata() {
        let tool = TodoWriteTool::new();
        assert_eq!(tool.name(), "todo_write");
        assert_eq!(tool.tier(), RiskTier::Safe);
    }

    #[tokio::test]
    async fn test_recording_todos_reports_the_count() {
        let tool = TodoWriteTool::new();
        let outcome = tool
            .invoke(
                todos(&[
                    ("research", TodoStatus::Pending),
                    ("write", TodoStatus::Pending),
                ]),
                &ctx(ConversationId(1)),
            )
            .await;

        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains('2'), "got {text:?}"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_current_returns_the_stored_list() {
        let tool = TodoWriteTool::new();
        tool.invoke(
            todos(&[("research", TodoStatus::InProgress)]),
            &ctx(ConversationId(1)),
        )
        .await;

        let stored = tool.current(Some(ConversationId(1)), None);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].content, "research");
        assert_eq!(stored[0].status, TodoStatus::InProgress);
    }

    #[tokio::test]
    async fn test_a_second_call_replaces_rather_than_appends() {
        let tool = TodoWriteTool::new();
        let id = ConversationId(1);

        tool.invoke(todos(&[("first", TodoStatus::Pending)]), &ctx(id))
            .await;
        tool.invoke(todos(&[("second", TodoStatus::Pending)]), &ctx(id))
            .await;

        let stored = tool.current(Some(id), None);
        assert_eq!(
            stored.len(),
            1,
            "the second call should replace, not append"
        );
        assert_eq!(stored[0].content, "second");
    }

    #[tokio::test]
    async fn test_different_conversations_have_independent_lists() {
        let tool = TodoWriteTool::new();

        tool.invoke(
            todos(&[("a", TodoStatus::Pending)]),
            &ctx(ConversationId(1)),
        )
        .await;
        tool.invoke(
            todos(&[("b", TodoStatus::Pending)]),
            &ctx(ConversationId(2)),
        )
        .await;

        assert_eq!(tool.current(Some(ConversationId(1)), None)[0].content, "a");
        assert_eq!(tool.current(Some(ConversationId(2)), None)[0].content, "b");
    }

    /// The scratchpad's whole reason to key on a `(conversation_id,
    /// pipeline_id)` pair rather than either alone: a pipeline whose
    /// numeric id happens to equal a real conversation's id must never
    /// share that conversation's own list - the exact collision a
    /// fabricated `ConversationId(pipeline_id.0)` used to risk (see
    /// `crate::pipeline::dispatch::AgentContext`'s own doc comment).
    #[tokio::test]
    async fn test_a_pipeline_and_a_conversation_with_the_same_numeric_id_never_collide() {
        let tool = TodoWriteTool::new();

        tool.invoke(
            todos(&[("chat task", TodoStatus::Pending)]),
            &ctx(ConversationId(1)),
        )
        .await;
        tool.invoke(
            todos(&[("pipeline task", TodoStatus::Pending)]),
            &pipeline_ctx(1),
        )
        .await;

        assert_eq!(
            tool.current(Some(ConversationId(1)), None)[0].content,
            "chat task"
        );
        assert_eq!(tool.current(None, Some(1))[0].content, "pipeline task");
    }

    #[tokio::test]
    async fn test_unknown_conversation_returns_an_empty_list() {
        let tool = TodoWriteTool::new();
        assert!(tool.current(Some(ConversationId(99)), None).is_empty());
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let tool = TodoWriteTool::new();
        let outcome = tool
            .invoke(json!({"todos": "not a list"}), &ctx(ConversationId(1)))
            .await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[test]
    fn test_input_schema_has_a_todos_property() {
        let schema = TodoWriteTool::new().input_schema();
        assert!(schema["properties"].get("todos").is_some());
    }
}
