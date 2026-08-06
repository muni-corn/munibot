use crate::{persona::PersonaId, tools::ToolCtx, types::AiError};

/// Runs one turn for a named persona, given a task brief rather than a
/// conversation - what the `delegate` tool calls to bring a specialist in.
///
/// A trait rather than depending on `Ai` directly: `Ai` owns the
/// `ToolRegistry` that the `delegate` tool is itself registered in, so a
/// direct dependency would be a real cycle (`Ai::turn -> Harness ->
/// ToolRegistry -> DelegateTool -> Ai::turn`, for the nested persona). This
/// inverts it exactly the way [`crate::provider::ProviderSource`] (called
/// `ProviderSource` there, defined in `service.rs`) inverted provider
/// resolution - the difference here is this trait's own consumer lives in
/// `tools`, not `service`, so it is defined here instead. Also means a test
/// can substitute a fake delegator returning a canned result, with no
/// provider and no network.
#[async_trait::async_trait]
pub trait Delegator: Send + Sync {
    /// Runs one turn for `persona`, treating `task` as its entire input -
    /// never the invoking conversation's own history, a real
    /// prompt-injection boundary as much as a cost one. Bounded by
    /// `ctx.remaining_budget`; refusing an unknown persona, a
    /// non-delegable one, or one past the depth cap is the `delegate`
    /// tool's own job, checked before this is ever called.
    ///
    /// Returns the specialist's final text - never a structured handoff:
    /// chat delegation only ever reaches a persona with no `handoff`
    /// configured.
    async fn delegate(
        &self,
        persona: &PersonaId,
        task: String,
        ctx: &ToolCtx,
    ) -> Result<String, AiError>;
}
