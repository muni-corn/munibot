//! Bounding how many pipelines run at once, globally and per repository,
//! and tracking which ones are currently running so one can be aborted.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use munibot_vcs::RepoRef;
use tokio_util::sync::CancellationToken;

use crate::pipeline::{PipelineId, SandboxLifecycle};

/// How many pipelines may run at once.
///
/// `per_repo_max` exists specifically so one runaway repository (an issue
/// that keeps triggering, a maintainer relabelling the same issue
/// repeatedly) cannot starve every other repository munibot watches of
/// its own share of `global_max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConcurrencyConfig {
    pub global_max: usize,
    pub per_repo_max: usize,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            global_max: 10,
            per_repo_max: 2,
        }
    }
}

/// Enforces a [`ConcurrencyConfig`] against how many pipelines are
/// currently running, globally and per repository.
struct ConcurrencyLimiter {
    config: ConcurrencyConfig,
    global_count: Mutex<usize>,
    per_repo_count: Mutex<HashMap<RepoRef, usize>>,
}

impl ConcurrencyLimiter {
    fn new(config: ConcurrencyConfig) -> Self {
        Self {
            config,
            global_count: Mutex::new(0),
            per_repo_count: Mutex::new(HashMap::new()),
        }
    }

    /// Reserves a slot for `repo`, or returns `None` if either the global
    /// or the per-repository maximum is already reached.
    fn try_acquire(self: &Arc<Self>, repo: &RepoRef) -> Option<ConcurrencyPermit> {
        let mut global_count = self.global_count.lock().expect("limiter lock poisoned");
        let mut per_repo_count = self.per_repo_count.lock().expect("limiter lock poisoned");
        let repo_count = per_repo_count.get(repo).copied().unwrap_or(0);

        if *global_count >= self.config.global_max || repo_count >= self.config.per_repo_max {
            return None;
        }

        *global_count += 1;
        *per_repo_count.entry(repo.clone()).or_insert(0) += 1;

        Some(ConcurrencyPermit {
            limiter: self.clone(),
            repo: repo.clone(),
        })
    }

    fn release(&self, repo: &RepoRef) {
        let mut global_count = self.global_count.lock().expect("limiter lock poisoned");
        *global_count = global_count.saturating_sub(1);

        if let Some(count) = self
            .per_repo_count
            .lock()
            .expect("limiter lock poisoned")
            .get_mut(repo)
        {
            *count = count.saturating_sub(1);
        }
    }
}

/// A reserved concurrency slot, released automatically when dropped.
struct ConcurrencyPermit {
    limiter: Arc<ConcurrencyLimiter>,
    repo: RepoRef,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        self.limiter.release(&self.repo);
    }
}

/// A FIFO queue of pipelines waiting for a concurrency slot to free up.
#[derive(Default)]
struct PipelineQueue {
    entries: Mutex<VecDeque<PipelineId>>,
}

impl PipelineQueue {
    fn new() -> Self {
        Self::default()
    }

    fn enqueue(&self, pipeline_id: PipelineId) {
        self.entries
            .lock()
            .expect("queue lock poisoned")
            .push_back(pipeline_id);
    }

    fn dequeue(&self) -> Option<PipelineId> {
        self.entries
            .lock()
            .expect("queue lock poisoned")
            .pop_front()
    }

    fn len(&self) -> usize {
        self.entries.lock().expect("queue lock poisoned").len()
    }
}

/// One pipeline this process is currently running.
struct RunningPipeline {
    cancellation: CancellationToken,
    sandbox: Arc<dyn SandboxLifecycle>,
    // held only for its Drop -- releases the concurrency slot once this
    // entry is removed from `PipelineRegistry::running`
    _permit: ConcurrencyPermit,
}

/// Tracks every pipeline this process is currently running, enforcing a
/// [`ConcurrencyConfig`] and queuing overflow for later.
pub struct PipelineRegistry {
    limiter: Arc<ConcurrencyLimiter>,
    queue: PipelineQueue,
    running: Mutex<HashMap<PipelineId, RunningPipeline>>,
}

impl PipelineRegistry {
    pub fn new(config: ConcurrencyConfig) -> Self {
        Self {
            limiter: Arc::new(ConcurrencyLimiter::new(config)),
            queue: PipelineQueue::new(),
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Tries to reserve a concurrency slot for `pipeline_id` on `repo`.
    ///
    /// Queues `pipeline_id` for later and returns `None` if no slot is
    /// available right now, rather than starting it anyway -- the whole
    /// point of a limit is that it is never exceeded, even briefly.
    /// Returns the [`CancellationToken`] a started run's own `Executor`
    /// should run with when a slot was reserved.
    pub fn try_start(
        &self,
        pipeline_id: PipelineId,
        repo: &RepoRef,
        sandbox: Arc<dyn SandboxLifecycle>,
    ) -> Option<CancellationToken> {
        let permit = self.limiter.try_acquire(repo)?;
        let cancellation = CancellationToken::new();

        self.running
            .lock()
            .expect("registry lock poisoned")
            .insert(pipeline_id, RunningPipeline {
                cancellation: cancellation.clone(),
                sandbox,
                _permit: permit,
            });

        Some(cancellation)
    }

    /// Queues `pipeline_id` for a later attempt, once queried by whatever
    /// retries overflow -- called after `try_start` returns `None`.
    pub fn enqueue(&self, pipeline_id: PipelineId) {
        self.queue.enqueue(pipeline_id);
    }

    /// Takes the next queued pipeline, if any, for a fresh `try_start`
    /// attempt.
    pub fn dequeue(&self) -> Option<PipelineId> {
        self.queue.dequeue()
    }

    /// How many pipelines are currently queued, waiting for a slot.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Marks `pipeline_id` as no longer running, freeing its concurrency
    /// slot for whatever is queued behind it.
    pub fn finish(&self, pipeline_id: PipelineId) {
        self.running
            .lock()
            .expect("registry lock poisoned")
            .remove(&pipeline_id);
    }

    /// Whether `pipeline_id` is currently running in this process.
    pub fn is_running(&self, pipeline_id: PipelineId) -> bool {
        self.running
            .lock()
            .expect("registry lock poisoned")
            .contains_key(&pipeline_id)
    }

    /// Aborts a running pipeline: cancels its own turn (propagating into
    /// the harness, which already checks its `ToolCtx::cancellation` mid
    /// turn) and tears down its sandbox, stopping the container. Returns
    /// whether `pipeline_id` was actually running.
    pub async fn abort_pipeline(&self, pipeline_id: PipelineId) -> bool {
        let running = self
            .running
            .lock()
            .expect("registry lock poisoned")
            .remove(&pipeline_id);

        match running {
            Some(running) => {
                running.cancellation.cancel();
                running.sandbox.teardown().await;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use munibot_vcs::Forge;

    use super::*;
    use crate::{pipeline::NoSandbox, tools::ToolRegistry};

    fn repo(name: &str) -> RepoRef {
        RepoRef::new(Forge::GitHub, "musicaloft", name)
    }

    fn sandbox() -> Arc<dyn SandboxLifecycle> {
        Arc::new(NoSandbox::new(Arc::new(ToolRegistry::new())))
    }

    #[test]
    fn test_default_concurrency_config_has_sensible_limits() {
        let config = ConcurrencyConfig::default();
        assert!(config.global_max > 0);
        assert!(config.per_repo_max > 0);
        assert!(config.per_repo_max <= config.global_max);
    }

    #[test]
    fn test_try_start_succeeds_within_limits() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 10,
            per_repo_max: 10,
        });
        assert!(
            registry
                .try_start(PipelineId(1), &repo("munibot"), sandbox())
                .is_some()
        );
        assert!(registry.is_running(PipelineId(1)));
    }

    #[test]
    fn test_try_start_fails_once_the_global_max_is_reached() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 1,
            per_repo_max: 10,
        });
        assert!(
            registry
                .try_start(PipelineId(1), &repo("a"), sandbox())
                .is_some()
        );
        assert!(
            registry
                .try_start(PipelineId(2), &repo("b"), sandbox())
                .is_none()
        );
    }

    #[test]
    fn test_try_start_fails_once_the_per_repo_max_is_reached_even_under_the_global_max() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 10,
            per_repo_max: 1,
        });
        assert!(
            registry
                .try_start(PipelineId(1), &repo("a"), sandbox())
                .is_some()
        );
        assert!(
            registry
                .try_start(PipelineId(2), &repo("a"), sandbox())
                .is_none(),
            "a second pipeline for the same repo should be refused"
        );
        assert!(
            registry
                .try_start(PipelineId(3), &repo("b"), sandbox())
                .is_some(),
            "a different repo should be unaffected by the first repo's own limit"
        );
    }

    #[test]
    fn test_finishing_a_pipeline_frees_its_slot_for_the_same_repo() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 10,
            per_repo_max: 1,
        });
        registry
            .try_start(PipelineId(1), &repo("a"), sandbox())
            .unwrap();
        assert!(
            registry
                .try_start(PipelineId(2), &repo("a"), sandbox())
                .is_none()
        );

        registry.finish(PipelineId(1));
        assert!(
            registry
                .try_start(PipelineId(2), &repo("a"), sandbox())
                .is_some(),
            "finishing the first should free a slot for the second"
        );
    }

    #[test]
    fn test_finishing_a_pipeline_frees_its_slot_globally_too() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 1,
            per_repo_max: 10,
        });
        registry
            .try_start(PipelineId(1), &repo("a"), sandbox())
            .unwrap();
        assert!(
            registry
                .try_start(PipelineId(2), &repo("b"), sandbox())
                .is_none()
        );

        registry.finish(PipelineId(1));
        assert!(
            registry
                .try_start(PipelineId(2), &repo("b"), sandbox())
                .is_some()
        );
    }

    #[test]
    fn test_enqueue_and_dequeue_are_first_in_first_out() {
        let registry = PipelineRegistry::new(ConcurrencyConfig::default());
        registry.enqueue(PipelineId(1));
        registry.enqueue(PipelineId(2));

        assert_eq!(registry.dequeue(), Some(PipelineId(1)));
        assert_eq!(registry.dequeue(), Some(PipelineId(2)));
        assert_eq!(registry.dequeue(), None);
    }

    #[test]
    fn test_queue_len_reflects_pending_entries() {
        let registry = PipelineRegistry::new(ConcurrencyConfig::default());
        assert_eq!(registry.queue_len(), 0);
        registry.enqueue(PipelineId(1));
        assert_eq!(registry.queue_len(), 1);
        registry.dequeue();
        assert_eq!(registry.queue_len(), 0);
    }

    #[tokio::test]
    async fn test_abort_pipeline_cancels_the_token_and_tears_down_the_sandbox() {
        struct CountingSandbox {
            teardowns: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl SandboxLifecycle for CountingSandbox {
            async fn provision(&self) -> Result<Arc<ToolRegistry>, crate::pipeline::ExecutorError> {
                Ok(Arc::new(ToolRegistry::new()))
            }

            async fn teardown(&self) {
                self.teardowns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let registry = PipelineRegistry::new(ConcurrencyConfig::default());
        let counting_sandbox = Arc::new(CountingSandbox {
            teardowns: std::sync::atomic::AtomicUsize::new(0),
        });
        let cancellation = registry
            .try_start(PipelineId(1), &repo("a"), counting_sandbox.clone())
            .unwrap();

        assert!(!cancellation.is_cancelled());

        assert!(registry.abort_pipeline(PipelineId(1)).await);

        assert!(cancellation.is_cancelled());
        assert_eq!(
            counting_sandbox
                .teardowns
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(!registry.is_running(PipelineId(1)));
    }

    #[tokio::test]
    async fn test_abort_pipeline_returns_false_for_one_that_is_not_running() {
        let registry = PipelineRegistry::new(ConcurrencyConfig::default());
        assert!(!registry.abort_pipeline(PipelineId(999)).await);
    }

    #[tokio::test]
    async fn test_aborting_frees_the_concurrency_slot() {
        let registry = PipelineRegistry::new(ConcurrencyConfig {
            global_max: 10,
            per_repo_max: 1,
        });
        registry
            .try_start(PipelineId(1), &repo("a"), sandbox())
            .unwrap();
        assert!(
            registry
                .try_start(PipelineId(2), &repo("a"), sandbox())
                .is_none()
        );

        registry.abort_pipeline(PipelineId(1)).await;
        assert!(
            registry
                .try_start(PipelineId(2), &repo("a"), sandbox())
                .is_some()
        );
    }
}
