use async_trait::async_trait;

use crate::{
    memory::user_memory::{Memory, MemoryStore},
    types::AiError,
};

/// Whether a person has opted into [`MemoryStore`].
///
/// Kept separate from `MemoryStore` itself: opt-in status is a fact about a
/// *person*, not about their memories, and [`GatedMemoryStore`] is what wires
/// the two together.
#[async_trait]
pub trait MemoryOptIn: Send + Sync {
    /// Whether `user_id` has opted in. `false` for a person who has never
    /// touched the setting - memory is opt-in, never assumed.
    async fn is_opted_in(&self, user_id: u64) -> Result<bool, AiError>;

    /// Sets whether `user_id` has opted in.
    async fn set_opted_in(&self, user_id: u64, opted_in: bool) -> Result<(), AiError>;
}

/// Wraps any [`MemoryStore`] with opt-in gating, so no implementation of that
/// trait needs to check `memory_opt_in` itself - enforced here once, in the
/// store, rather than trusted to every future caller to remember.
///
/// `list`, `record`, and `forget` are all gated: `list` returns empty rather
/// than an error for someone who has not opted in, since it is read-only and
/// "nothing recorded" is simply true from their perspective; `record` and
/// `forget` return a clear, recoverable refusal, since attempting either is
/// an explicit request the gate should explain rather than silently no-op.
///
/// `wipe` is **not** gated. It is never offered to a model as a tool (there
/// is deliberately no `wipe` tool, only a human-facing "delete everything"
/// action in the eventual memory panel), so the gate's actual purpose -
/// stopping a model from quietly building up memory on someone who never
/// consented - does not apply to it. A person who opted in, recorded something,
/// and then opted back out still has every right to delete what is left over,
/// without having to opt back in first just to do it.
pub struct GatedMemoryStore<S, O> {
    inner: S,
    opt_in: O,
}

impl<S: MemoryStore, O: MemoryOptIn> GatedMemoryStore<S, O> {
    pub fn new(inner: S, opt_in: O) -> Self {
        Self { inner, opt_in }
    }
}

#[async_trait]
impl<S: MemoryStore, O: MemoryOptIn> MemoryStore for GatedMemoryStore<S, O> {
    async fn list(&self, user_id: u64) -> Result<Vec<Memory>, AiError> {
        if !self.opt_in.is_opted_in(user_id).await? {
            return Ok(Vec::new());
        }
        self.inner.list(user_id).await
    }

    async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError> {
        if !self.opt_in.is_opted_in(user_id).await? {
            return Err(AiError::Config(
                "memory is off :< opt in first to have me remember things".to_string(),
            ));
        }
        self.inner.record(user_id, key, value).await
    }

    async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError> {
        if !self.opt_in.is_opted_in(user_id).await? {
            return Err(AiError::Config(
                "memory is off :< there's nothing to forget".to_string(),
            ));
        }
        self.inner.forget(user_id, key).await
    }

    async fn wipe(&self, user_id: u64) -> Result<(), AiError> {
        self.inner.wipe(user_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A [`MemoryStore`] that records every call it received, for asserting
    /// the gate did or did not delegate to it.
    #[derive(Default)]
    struct FakeMemoryStore {
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl MemoryStore for FakeMemoryStore {
        async fn list(&self, _user_id: u64) -> Result<Vec<Memory>, AiError> {
            self.calls.lock().unwrap().push("list");
            Ok(vec![Memory {
                key: "k".to_string(),
                value: "v".to_string(),
                updated_at: chrono::Utc::now(),
            }])
        }

        async fn record(&self, _user_id: u64, _key: &str, _value: &str) -> Result<(), AiError> {
            self.calls.lock().unwrap().push("record");
            Ok(())
        }

        async fn forget(&self, _user_id: u64, _key: &str) -> Result<(), AiError> {
            self.calls.lock().unwrap().push("forget");
            Ok(())
        }

        async fn wipe(&self, _user_id: u64) -> Result<(), AiError> {
            self.calls.lock().unwrap().push("wipe");
            Ok(())
        }
    }

    /// A [`MemoryOptIn`] returning a fixed answer, for tests that do not care
    /// about persistence.
    struct FixedOptIn(bool);

    #[async_trait]
    impl MemoryOptIn for FixedOptIn {
        async fn is_opted_in(&self, _user_id: u64) -> Result<bool, AiError> {
            Ok(self.0)
        }

        async fn set_opted_in(&self, _user_id: u64, _opted_in: bool) -> Result<(), AiError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_list_returns_empty_without_delegating_when_not_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(false));
        let memories = gate.list(1).await.expect("should succeed");

        assert!(memories.is_empty());
        assert!(
            gate.inner.calls.lock().unwrap().is_empty(),
            "list should never even reach the inner store when not opted in"
        );
    }

    #[tokio::test]
    async fn test_list_delegates_when_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(true));
        let memories = gate.list(1).await.expect("should succeed");

        assert_eq!(memories.len(), 1);
        assert_eq!(*gate.inner.calls.lock().unwrap(), vec!["list"]);
    }

    #[tokio::test]
    async fn test_record_is_refused_without_delegating_when_not_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(false));
        let result = gate.record(1, "k", "v").await;

        assert!(result.is_err());
        assert!(gate.inner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_record_delegates_when_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(true));
        gate.record(1, "k", "v").await.expect("should succeed");

        assert_eq!(*gate.inner.calls.lock().unwrap(), vec!["record"]);
    }

    #[tokio::test]
    async fn test_forget_is_refused_without_delegating_when_not_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(false));
        let result = gate.forget(1, "k").await;

        assert!(result.is_err());
        assert!(gate.inner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_forget_delegates_when_opted_in() {
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(true));
        gate.forget(1, "k").await.expect("should succeed");

        assert_eq!(*gate.inner.calls.lock().unwrap(), vec!["forget"]);
    }

    #[tokio::test]
    async fn test_wipe_delegates_even_when_not_opted_in() {
        // the one deliberate exception: deleting your own leftover data must never
        // require opting back in first
        let gate = GatedMemoryStore::new(FakeMemoryStore::default(), FixedOptIn(false));
        gate.wipe(1).await.expect("wipe must never be gated");

        assert_eq!(*gate.inner.calls.lock().unwrap(), vec!["wipe"]);
    }
}
