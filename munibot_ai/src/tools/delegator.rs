use std::sync::{Arc, OnceLock, Weak};

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

/// A [`Delegator`] set once, after the `Ai` that will implement it actually
/// exists.
///
/// The `delegate` tool is registered into a [`crate::tools::ToolRegistry`]
/// that `Ai::new` itself takes as a constructor argument, so there is no
/// `Ai` yet at the point the tool is built. Holding a [`Weak`] rather than an
/// [`Arc`] once set matters too: `Ai` owns the registry the tool lives in, so
/// an `Arc` in both directions would be a real reference cycle neither side
/// ever gets dropped from - upgrading a `Weak` at call time costs nothing an
/// `Arc` clone wouldn't anyway, since `Ai` is expected to outlive every turn
/// that could possibly call this.
#[derive(Default)]
pub struct DelegatorCell(OnceLock<Weak<dyn Delegator>>);

impl DelegatorCell {
    pub fn new() -> Self {
        Self::default()
    }

    /// Completes the wiring, once `Ai` exists. A no-op if called more than
    /// once - this is meant to be set exactly once, at startup.
    pub fn set(&self, delegator: Weak<dyn Delegator>) {
        let _ = self.0.set(delegator);
    }

    /// The live delegator, if [`Self::set`] has run and its target still
    /// exists.
    pub fn get(&self) -> Option<Arc<dyn Delegator>> {
        self.0.get()?.upgrade()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeDelegator;

    #[async_trait::async_trait]
    impl Delegator for FakeDelegator {
        async fn delegate(
            &self,
            _persona: &PersonaId,
            _task: String,
            _ctx: &ToolCtx,
        ) -> Result<String, AiError> {
            Ok("canned".to_string())
        }
    }

    #[test]
    fn test_an_unset_cell_has_no_delegator() {
        let cell = DelegatorCell::new();
        assert!(cell.get().is_none());
    }

    #[test]
    fn test_get_returns_the_delegator_once_set() {
        let cell = DelegatorCell::new();
        let delegator: Arc<dyn Delegator> = Arc::new(FakeDelegator);

        cell.set(Arc::downgrade(&delegator));

        assert!(cell.get().is_some());
    }

    #[test]
    fn test_get_returns_none_once_the_target_has_been_dropped() {
        let cell = DelegatorCell::new();
        let delegator: Arc<dyn Delegator> = Arc::new(FakeDelegator);
        cell.set(Arc::downgrade(&delegator));

        drop(delegator);

        assert!(
            cell.get().is_none(),
            "a dropped target should not resurrect as a strong reference"
        );
    }

    #[test]
    fn test_setting_twice_keeps_the_first_value() {
        let cell = DelegatorCell::new();
        let first: Arc<dyn Delegator> = Arc::new(FakeDelegator);
        let second: Arc<dyn Delegator> = Arc::new(FakeDelegator);

        cell.set(Arc::downgrade(&first));
        cell.set(Arc::downgrade(&second));

        assert!(
            cell.get().is_some_and(|got| Arc::ptr_eq(&got, &first)),
            "the second set should be a no-op"
        );
    }
}
