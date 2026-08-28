//! Provisions a sandbox for one turn, according to a persona's
//! [`SandboxPolicy`].

use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::{
    persona::SandboxPolicy,
    sandbox::{
        config::SandboxConfig,
        container::Sandbox,
        tools::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, SandboxSession, WriteTool},
    },
    tools::{Tool, ToolRegistry},
    types::AiError,
};

/// How long [`provision_if_needed`] waits for the tool agent's socket to
/// become connectable after starting the container.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A sandbox provisioned for one turn: the six sandbox tools already
/// layered onto whatever base registry the turn would otherwise use.
///
/// Holding this alive for the turn's own duration - never storing it on
/// `Ai` itself - is what "tears down afterwards" means: the container and
/// its host-side workspace directory are cleaned up (the container via
/// [`Sandbox`]'s own best-effort `Drop`, the workspace directory via this
/// type's) the moment nothing references this any more.
pub struct ProvisionedSandbox {
    /// The base registry with the six sandbox tools added on top.
    pub tools: Arc<ToolRegistry>,
    // ordering matters for Drop: the container is torn down before the
    // workspace directory it was bind-mounted from is removed out from
    // under it, since struct fields drop top-to-bottom
    _sandbox: Sandbox,
    _workspace: WorkspaceGuard,
}

/// Removes the host-side checkout directory a [`Sandbox`] borrowed as its
/// workspace mount, once nothing needs it any more.
///
/// [`Sandbox`] itself only ever cleans up the socket directory it
/// generated - the workspace directory came from the caller (this
/// module), so cleaning it up is this module's job too.
struct WorkspaceGuard(PathBuf);

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Provisions a sandbox and layers its six tools onto `base_tools`, unless
/// `policy` is [`SandboxPolicy::Forbidden`].
///
/// [`SandboxPolicy::Optional`] is provisioned exactly as eagerly as
/// [`SandboxPolicy::Required`] here - true lazy provisioning (nothing
/// created until the first sandbox tool call actually happens) is a scoped
/// -down simplification tracked in
/// `docs/notes/sandbox-verification-gaps.md`, not yet built.
///
/// Nothing is checked out into the workspace here - that is
/// [`Sandbox::checkout`]'s job, and needs a repository to check out that
/// this function has no source for yet (a chat-delegated turn has no
/// issue or pull request behind it; that context arrives with the
/// pipeline in milestone 5). A sandbox provisioned this way starts with an
/// empty workspace a persona can still `write`/`bash` its way through.
pub async fn provision_if_needed(
    policy: SandboxPolicy,
    config: SandboxConfig,
    base_tools: &Arc<ToolRegistry>,
) -> Result<Option<ProvisionedSandbox>, AiError> {
    if policy == crate::persona::SandboxPolicy::Forbidden {
        return Ok(None);
    }

    let workspace = std::env::temp_dir().join(format!(
        "munibot-sandbox-workspace-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).map_err(|error| {
        AiError::Other(format!("couldn't prepare a sandbox workspace :< {error}"))
    })?;
    let workspace_guard = WorkspaceGuard(workspace.clone());

    let mut sandbox = Sandbox::new(config)?.with_workspace_mount(&workspace);
    let cmd = sandbox.tool_agent_cmd();
    sandbox.create(cmd).await?;
    sandbox.start().await?;
    let client = Arc::new(sandbox.connect_tool_agent(CONNECT_TIMEOUT).await?);

    let session = SandboxSession::new(client);
    let tools = Arc::new(base_tools.with_overlay(sandboxed_tools(session)));

    Ok(Some(ProvisionedSandbox {
        tools,
        _sandbox: sandbox,
        _workspace: workspace_guard,
    }))
}

fn sandboxed_tools(session: Arc<SandboxSession>) -> [Arc<dyn Tool>; 6] {
    [
        Arc::new(ReadTool::new(Arc::clone(&session))),
        Arc::new(WriteTool::new(Arc::clone(&session))),
        Arc::new(EditTool::new(Arc::clone(&session))),
        Arc::new(GrepTool::new(Arc::clone(&session))),
        Arc::new(GlobTool::new(Arc::clone(&session))),
        Arc::new(BashTool::new(session)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_forbidden_policy_provisions_nothing() {
        let base = Arc::new(ToolRegistry::new());
        let result = provision_if_needed(SandboxPolicy::Forbidden, SandboxConfig::default(), &base)
            .await
            .expect("should succeed");
        assert!(result.is_none());
    }

    /// Combines both assertions - the connect failure itself, and that its
    /// scratch workspace directory is cleaned up regardless - in one test
    /// rather than two, since counting `munibot-sandbox-workspace-*`
    /// directories in the shared system temp dir is inherently racy
    /// against any *other* test doing the same thing concurrently.
    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_failed_provision_still_cleans_up_its_workspace_directory() {
        let base = Arc::new(ToolRegistry::new());
        let config = SandboxConfig {
            image: "alpine:latest".to_string(),
            ..Default::default()
        };

        let workspaces_before = matching_workspace_dirs();

        // alpine has no munibot_toolagent baked in, so connecting will
        // fail - this exercises provisioning up through container start,
        // not a full working tool agent (that needs the real
        // Containerfile image; see docs/notes/sandbox-verification-gaps.md)
        let result = provision_if_needed(SandboxPolicy::Required, config, &base).await;
        assert!(
            result.is_err(),
            "connecting to a tool agent that was never started should fail cleanly, not hang"
        );

        let new_workspaces: Vec<_> = matching_workspace_dirs()
            .into_iter()
            .filter(|dir| !workspaces_before.contains(dir))
            .collect();
        assert!(
            new_workspaces.is_empty(),
            "a failed provision should leave no scratch workspace directory behind, found \
             {new_workspaces:?}"
        );
    }

    fn matching_workspace_dirs() -> Vec<std::ffi::OsString> {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| {
                name.to_string_lossy()
                    .starts_with("munibot-sandbox-workspace-")
            })
            .collect()
    }
}
