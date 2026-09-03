//! `WorkspaceSession`: provisions a sandbox container lazily, on the
//! first sandboxed tool call that actually needs one, rather than
//! eagerly at construction.

use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio::sync::OnceCell;

use crate::{
    sandbox::{config::SandboxConfig, container::Sandbox, tools::SandboxSession},
    types::AiError,
};

/// How long [`WorkspaceSession::sandbox_session`] waits for the tool
/// agent's socket to become connectable after starting the container.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the workspace directory a [`Sandbox`] borrows as its mount
/// should be removed once nothing needs this session any more.
///
/// `Scratch` - a fresh temp directory [`WorkspaceSession::new`] created
/// for exactly this session - is cleaned up on drop, the same as before
/// project-backed workspaces existed. `Persistent` - an existing worktree
/// [`WorkspaceSession::over_workspace`] was pointed at, owned by a
/// project's own `ai_workspaces` row - is left alone: a persistent
/// workspace surviving the session that used it is the entire point of
/// it being persistent (see `docs/plans/ai/milestone-7-projects.md`'s
/// architecture section).
enum WorkspaceGuard {
    Scratch(PathBuf),
    Persistent,
}

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        if let Self::Scratch(path) = self {
            std::fs::remove_dir_all(path).ok();
        }
    }
}

/// What [`WorkspaceSession::sandbox_session`] provisions and then holds
/// alive for as long as the session lives.
///
/// `_sandbox` is `None` only in tests built through
/// [`WorkspaceSession::already_connected`], which wraps an already-live
/// [`SandboxSession`] (talking to a fake agent, typically) with no real
/// container underneath it at all - see that constructor's own doc
/// comment for why tool-behavior tests need this seam.
struct ProvisionedConnection {
    session: Arc<SandboxSession>,
    _sandbox: Option<Sandbox>,
}

/// A sandbox provisioned lazily, on the first sandboxed tool call that
/// actually needs one.
///
/// Two consequences of this laziness, both load-bearing for gap 9 (see
/// `docs/plans/ai/milestone-7-projects.md`, commit 218):
///
/// - The six sandbox tools (`crate::sandbox::tools`) are always present in the
///   schema whenever a persona's policy is not `Forbidden`, since constructing
///   a `WorkspaceSession` does nothing but reserve a scratch directory - the
///   model's own view of its capabilities never depends on provisioning timing.
/// - A provisioning failure surfaces as a tool error on the *first call*, not a
///   turn failure: the model can read "podman isn't available" and adapt, which
///   is strictly better than the turn dying before it has said a word.
///
/// Holding this alive for a turn's own duration is what "tears down
/// afterwards" means: the container - via [`Sandbox`]'s own best-effort
/// `Drop` - and the host-side workspace directory are cleaned up the
/// moment nothing references this any more.
///
/// Exactly one container is ever created no matter how many sandboxed
/// tool calls race to be the first: [`tokio::sync::OnceCell::get_or_try_init`]
/// guarantees every concurrent caller awaits the very same initialization
/// future.
pub struct WorkspaceSession {
    config: SandboxConfig,
    workspace: PathBuf,
    // declared before `_workspace_guard` so it drops first: the
    // container (holding a bind mount into the workspace directory) must
    // be gone before that directory is removed out from under it
    cell: OnceCell<ProvisionedConnection>,
    _workspace_guard: WorkspaceGuard,
}

impl WorkspaceSession {
    /// Reserves a scratch workspace directory - cheap, and safe to do
    /// even if nothing ever actually calls [`Self::sandbox_session`] -
    /// but creates no container yet. Removed once this session is
    /// dropped - see [`WorkspaceGuard::Scratch`].
    pub fn new(config: SandboxConfig) -> Result<Arc<Self>, AiError> {
        let workspace = fresh_workspace_dir("munibot-sandbox-workspace")?;
        Ok(Arc::new(Self {
            config,
            workspace: workspace.clone(),
            cell: OnceCell::new(),
            _workspace_guard: WorkspaceGuard::Scratch(workspace),
        }))
    }

    /// Provisions lazily over an existing, persistent workspace directory
    /// (a project's own worktree, from `ProjectManager::workspace_for`)
    /// rather than a fresh scratch one.
    ///
    /// Never removes `workspace`: it outlives this session by design,
    /// and belongs to whatever `ai_workspaces` row created it.
    pub fn over_workspace(workspace: PathBuf, config: SandboxConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            workspace,
            cell: OnceCell::new(),
            _workspace_guard: WorkspaceGuard::Persistent,
        })
    }

    /// Wraps an already-connected [`SandboxSession`] with no real
    /// container behind it at all - what tool-behavior tests use to
    /// exercise real request/response marshalling and the
    /// read-before-write guard against a fake agent, without needing
    /// rootless podman at all. Never used by production code, which
    /// always goes through [`Self::new`].
    #[cfg(test)]
    pub(crate) fn already_connected(session: Arc<SandboxSession>) -> Arc<Self> {
        let workspace = fresh_workspace_dir("munibot-sandbox-workspace-test")
            .expect("test tmpdir should be writable");
        let cell = OnceCell::new();
        cell.set(ProvisionedConnection {
            session,
            _sandbox: None,
        })
        .ok();
        Arc::new(Self {
            config: SandboxConfig::default(),
            workspace: workspace.clone(),
            cell,
            _workspace_guard: WorkspaceGuard::Scratch(workspace),
        })
    }

    /// The connected sandbox session, provisioning a real container on
    /// the first call - every later call reuses the exact same
    /// connection.
    pub async fn sandbox_session(&self) -> Result<Arc<SandboxSession>, AiError> {
        let connection = self
            .cell
            .get_or_try_init(|| async {
                let mut sandbox =
                    Sandbox::new(self.config.clone())?.with_workspace_mount(&self.workspace);
                let cmd = sandbox.tool_agent_cmd();
                sandbox.create(cmd).await?;
                sandbox.start().await?;
                let client = Arc::new(sandbox.connect_tool_agent(CONNECT_TIMEOUT).await?);
                Ok::<_, AiError>(ProvisionedConnection {
                    session: SandboxSession::new(client),
                    _sandbox: Some(sandbox),
                })
            })
            .await?;
        Ok(connection.session.clone())
    }
}

fn fresh_workspace_dir(prefix: &str) -> Result<PathBuf, AiError> {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).map_err(|error| {
        AiError::Other(format!("couldn't prepare a sandbox workspace :< {error}"))
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::config::NetworkPolicy;

    fn bad_config() -> SandboxConfig {
        SandboxConfig {
            image: "definitely-does-not-exist-anywhere:latest".to_string(),
            cpu_quota: 0.5,
            memory_limit_bytes: 64 * 1024 * 1024,
            pids_limit: 32,
            disk_limit_bytes: None,
            network: NetworkPolicy::None,
            wall_clock_limit: std::time::Duration::from_secs(30),
            tool_agent_path: None,
        }
    }

    #[test]
    fn test_new_reserves_a_workspace_directory_without_touching_podman_at_all() {
        let session = WorkspaceSession::new(bad_config()).expect("should succeed");
        assert!(session.workspace.is_dir());
    }

    #[tokio::test]
    async fn test_dropping_before_first_use_removes_the_workspace_directory() {
        let session = WorkspaceSession::new(bad_config()).expect("should succeed");
        let workspace = session.workspace.clone();
        assert!(workspace.is_dir());
        drop(session);
        assert!(!workspace.is_dir());
    }

    #[tokio::test]
    async fn test_dropping_a_persistent_workspace_session_never_removes_the_directory() {
        let workspace = fresh_workspace_dir("munibot_ai_session_persistent_test").unwrap();
        let session = WorkspaceSession::over_workspace(workspace.clone(), bad_config());
        drop(session);
        assert!(
            workspace.is_dir(),
            "a project's own persistent workspace must survive the session that used it"
        );
        std::fs::remove_dir_all(&workspace).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_dropping_after_a_failed_provisioning_attempt_still_removes_the_workspace_directory()
     {
        let session = WorkspaceSession::new(bad_config()).expect("should succeed");
        let workspace = session.workspace.clone();

        if session.sandbox_session().await.is_ok() {
            panic!("an image that doesn't exist should fail to provision");
        }
        assert!(
            workspace.is_dir(),
            "still there right after the failed attempt"
        );

        drop(session);
        assert!(
            !workspace.is_dir(),
            "a failed provisioning attempt must not leave the scratch workspace behind"
        );
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_sandbox_session_fails_cleanly_for_an_image_that_does_not_exist() {
        let session = WorkspaceSession::new(bad_config()).expect("should succeed");
        let Err(error) = session.sandbox_session().await else {
            panic!("an image that doesn't exist should fail to provision");
        };
        assert!(!error.to_string().is_empty());
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_many_concurrent_first_calls_provision_exactly_one_container() {
        let config = SandboxConfig {
            image: "alpine:latest".to_string(),
            ..bad_config()
        };
        let session = WorkspaceSession::new(config).expect("should succeed");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let session = Arc::clone(&session);
            handles.push(tokio::spawn(async move { session.sandbox_session().await }));
        }

        // every call connects to the tool agent, which alpine has never
        // heard of - each individually fails, but they must all fail
        // identically, having raced for the very same underlying
        // provisioning attempt rather than each starting their own
        // container
        let mut messages = std::collections::HashSet::new();
        for handle in handles {
            let Err(error) = handle.await.unwrap() else {
                panic!("alpine has no tool agent to connect to");
            };
            messages.insert(error.to_string());
        }
        assert_eq!(
            messages.len(),
            1,
            "every concurrent caller should see the exact same failure from the exact same attempt"
        );
    }
}
