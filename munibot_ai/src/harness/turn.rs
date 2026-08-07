use serde_json::Value;

use crate::{
    harness::Budget,
    tools::{ToolCtx, ToolSelection},
    types::{Cost, History, ModelParams, ModelRef, Usage},
};

/// The structured payload a turn must produce to end via handoff, rather than
/// plain text.
#[derive(Clone, Debug)]
pub struct HandoffSchema {
    /// The name of the tool injected into the request so the model can hand
    /// off. Conventionally `"handoff"`.
    pub tool_name: String,
    /// When and how to use the injected tool, shown to the model verbatim. This
    /// is not documentation - it is the only thing telling the model what a
    /// valid handoff looks like for this particular role, so a generic
    /// description hurts every persona that uses one.
    pub description: String,
    /// The JSON Schema the handoff payload must satisfy.
    pub schema: Value,
}

impl HandoffSchema {
    /// Builds a handoff schema with the conventional tool name.
    pub fn new(description: impl Into<String>, schema: Value) -> Self {
        Self {
            tool_name: "handoff".to_string(),
            description: description.into(),
            schema,
        }
    }
}

/// Everything needed to run one full agent turn.
pub struct TurnRequest {
    pub system: Option<String>,
    pub history: History,
    pub tools: ToolSelection,
    pub model: ModelRef,
    pub params: ModelParams,
    pub budget: Budget,
    /// When set, the turn only ends by producing this handoff payload, never
    /// plain text.
    pub handoff: Option<HandoffSchema>,
    pub ctx: ToolCtx,
}

impl TurnRequest {
    /// Builds a request with no system prompt, no tools selected, default
    /// parameters and budget, and no handoff requirement.
    pub fn new(model: ModelRef, history: History, ctx: ToolCtx) -> Self {
        Self {
            system: None,
            history,
            tools: ToolSelection::none(),
            model,
            params: ModelParams::default(),
            budget: Budget::default(),
            handoff: None,
            ctx,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_tools(mut self, tools: ToolSelection) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_params(mut self, params: ModelParams) -> Self {
        self.params = params;
        self
    }

    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    pub fn with_handoff(mut self, handoff: HandoffSchema) -> Self {
        self.handoff = Some(handoff);
        self
    }
}

/// What a turn spent, regardless of whether it succeeded.
///
/// [`TurnOutcome`] already carries this on the success path; this type exists
/// for [`super::Harness::run_turn_recording_usage`], so a caller can still
/// learn what a *failed* turn cost - a turn that errored on its ninth
/// iteration still spent the first eight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnUsage {
    pub usage: Usage,
    pub cost: Cost,
    pub iterations: usize,
}

/// The result of one completed turn.
///
/// A turn ends with either `text` or `handoff` populated, never both - enforced
/// by the harness loop rather than by this type, so a partially-built outcome
/// mid-construction is not forced through an awkward enum.
#[derive(Debug)]
pub struct TurnOutcome {
    /// The assistant's final text, when the turn ended that way rather than via
    /// handoff.
    pub text: Option<String>,
    /// The validated handoff payload, when the turn ended that way.
    pub handoff: Option<Value>,
    pub usage: Usage,
    pub cost: Cost,
    pub iterations: usize,
}

impl TurnOutcome {
    /// Builds a plain-text outcome.
    pub fn text(text: impl Into<String>, usage: Usage, cost: Cost, iterations: usize) -> Self {
        Self {
            text: Some(text.into()),
            handoff: None,
            usage,
            cost,
            iterations,
        }
    }

    /// Builds a handoff outcome.
    pub fn handoff(payload: Value, usage: Usage, cost: Cost, iterations: usize) -> Self {
        Self {
            text: None,
            handoff: Some(payload),
            usage,
            cost,
            iterations,
        }
    }

    /// Returns `true` if this turn ended via a handoff rather than plain text.
    pub fn is_handoff(&self) -> bool {
        self.handoff.is_some()
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
            granted_tier: crate::tools::RiskTier::Safe,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
            delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    fn model() -> ModelRef {
        ModelRef::new("anthropic", "claude-opus-5")
    }

    #[test]
    fn test_new_request_has_no_tools_selected() {
        let request = TurnRequest::new(model(), History::new(), ctx());
        assert!(
            !request
                .tools
                .covers("web_search", crate::tools::RiskTier::NetworkRead),
            "a fresh request should select no tools until with_tools is called"
        );
        assert!(request.handoff.is_none());
    }

    #[test]
    fn test_builders_accumulate() {
        let request = TurnRequest::new(model(), History::new(), ctx())
            .with_system("be nice")
            .with_tools(ToolSelection::all())
            .with_handoff(HandoffSchema::new(
                "finish the turn with this shape",
                json!({"type": "object"}),
            ));

        assert_eq!(request.system.as_deref(), Some("be nice"));
        assert!(
            request
                .tools
                .covers("anything", crate::tools::RiskTier::Safe)
        );
        assert!(request.handoff.is_some());
    }

    #[test]
    fn test_handoff_schema_uses_the_conventional_tool_name() {
        let handoff =
            HandoffSchema::new("finish the turn with this shape", json!({"type": "object"}));
        assert_eq!(handoff.tool_name, "handoff");
    }

    #[test]
    fn test_handoff_schema_keeps_its_own_description() {
        let handoff = HandoffSchema::new("approve or reject the plan", json!({"type": "object"}));
        assert_eq!(
            handoff.description, "approve or reject the plan",
            "each role's handoff needs its own description, not a generic one"
        );
    }

    #[test]
    fn test_text_outcome_is_not_a_handoff() {
        let outcome = TurnOutcome::text("hi", Usage::default(), Cost::ZERO, 1);
        assert!(!outcome.is_handoff());
        assert_eq!(outcome.text.as_deref(), Some("hi"));
    }

    #[test]
    fn test_handoff_outcome_is_a_handoff() {
        let outcome = TurnOutcome::handoff(
            json!({"action": "ApprovePlan"}),
            Usage::default(),
            Cost::ZERO,
            3,
        );
        assert!(outcome.is_handoff());
        assert!(outcome.text.is_none());
        assert_eq!(outcome.iterations, 3);
    }
}
