use std::sync::Arc;

use async_trait::async_trait;

use crate::{memory::MemoryStore, tools::MemoryBackend, types::AiError};

/// Adapts any [`MemoryStore`] to the narrower [`MemoryBackend`] interface the
/// `remember`/`forget` tools need.
///
/// `ai::tools` cannot name [`MemoryStore`] directly - it sits below
/// `ai::memory` in this crate's dependency graph - so this bridge lives here,
/// on the side that is already allowed to depend on both.
pub struct MemoryToolBackend {
    store: Arc<dyn MemoryStore>,
}

impl MemoryToolBackend {
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl MemoryBackend for MemoryToolBackend {
    async fn record(&self, user_id: u64, key: &str, value: &str) -> Result<(), AiError> {
        self.store.record(user_id, key, value).await
    }

    async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError> {
        self.store.forget(user_id, key).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::memory::Memory;

    /// A [`MemoryStore`] recording every call, so these tests can assert the
    /// adapter forwarded to it rather than reimplementing anything itself.
    #[derive(Default)]
    struct FakeMemoryStore {
        calls: Mutex<Vec<(&'static str, u64, String)>>,
    }

    #[async_trait]
    impl MemoryStore for FakeMemoryStore {
        async fn list(&self, _user_id: u64) -> Result<Vec<Memory>, AiError> {
            Ok(Vec::new())
        }

        async fn record(&self, user_id: u64, key: &str, _value: &str) -> Result<(), AiError> {
            self.calls
                .lock()
                .unwrap()
                .push(("record", user_id, key.to_string()));
            Ok(())
        }

        async fn forget(&self, user_id: u64, key: &str) -> Result<(), AiError> {
            self.calls
                .lock()
                .unwrap()
                .push(("forget", user_id, key.to_string()));
            Ok(())
        }

        async fn wipe(&self, _user_id: u64) -> Result<(), AiError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_record_forwards_to_the_wrapped_store() {
        let store = Arc::new(FakeMemoryStore::default());
        let backend = MemoryToolBackend::new(store.clone());

        backend
            .record(7, "favorite_color", "purple")
            .await
            .expect("should succeed");

        assert_eq!(*store.calls.lock().unwrap(), vec![(
            "record",
            7,
            "favorite_color".to_string()
        )]);
    }

    #[tokio::test]
    async fn test_forget_forwards_to_the_wrapped_store() {
        let store = Arc::new(FakeMemoryStore::default());
        let backend = MemoryToolBackend::new(store.clone());

        backend
            .forget(7, "favorite_color")
            .await
            .expect("should succeed");

        assert_eq!(*store.calls.lock().unwrap(), vec![(
            "forget",
            7,
            "favorite_color".to_string()
        )]);
    }

    #[tokio::test]
    async fn test_a_store_error_propagates_through_the_adapter() {
        struct FailingStore;

        #[async_trait]
        impl MemoryStore for FailingStore {
            async fn list(&self, _user_id: u64) -> Result<Vec<Memory>, AiError> {
                Ok(Vec::new())
            }

            async fn record(&self, _user_id: u64, _key: &str, _value: &str) -> Result<(), AiError> {
                Err(AiError::Config("memory is off :<".to_string()))
            }

            async fn forget(&self, _user_id: u64, _key: &str) -> Result<(), AiError> {
                Ok(())
            }

            async fn wipe(&self, _user_id: u64) -> Result<(), AiError> {
                Ok(())
            }
        }

        let backend = MemoryToolBackend::new(Arc::new(FailingStore));
        let result = backend.record(1, "k", "v").await;
        assert!(result.is_err());
    }
}
