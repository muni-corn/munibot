//! Provisions a sandbox for one turn, according to a persona's
//! [`SandboxPolicy`].

use std::{path::PathBuf, sync::Arc};

use crate::{
    persona::SandboxPolicy,
    sandbox::{
        config::SandboxConfig,
        session::WorkspaceSession,
        tools::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, WriteTool},
    },
    tools::{Tool, ToolRegistry},
    types::AiError,
};

/// A sandbox provisioned for one turn: the six sandbox tools already
/// layered onto whatever base registry the turn would otherwise use,
/// bound to a [`WorkspaceSession`] that provisions lazily - see that
/// type's own doc comment for what "provisioned" means here and why
/// nothing is actually created yet.
///
/// Holding this alive for the turn's own duration - never storing it on
/// `Ai` itself - is what "tears down afterwards" means: whatever
/// container the session ends up provisioning, and its host-side
/// workspace directory, are cleaned up the moment nothing references this
/// any more.
pub struct ProvisionedSandbox {
    /// The base registry with the six sandbox tools added on top.
    pub tools: Arc<ToolRegistry>,
    session: Arc<WorkspaceSession>,
}

impl ProvisionedSandbox {
    /// Forces the underlying [`WorkspaceSession`] to provision right now,
    /// instead of waiting for the first sandboxed tool call. What
    /// [`crate::persona::SandboxPolicy::Required`] uses (see `Ai::prepare`)
    /// to keep failing the turn immediately when a sandbox can't actually
    /// be created, rather than only discovering that on the model's first
    /// tool call - the one piece of eagerness this milestone's own laziness
    /// deliberately keeps, because "required" means the persona cannot do
    /// its job at all without one.
    pub async fn force_provision(&self) -> Result<(), AiError> {
        self.session.sandbox_session().await?;
        Ok(())
    }
}

/// Adds the six sandbox tools, bound to a [`WorkspaceSession`], onto
/// `base_tools`, unless `policy` is [`SandboxPolicy::Forbidden`].
///
/// `workspace` chooses which kind of session that is: `None` reserves a
/// fresh scratch directory ([`WorkspaceSession::new`]), cleaned up the
/// moment nothing references this any more; `Some(path)` provisions
/// lazily over that existing, persistent directory instead
/// ([`WorkspaceSession::over_workspace`]) - a project's own worktree,
/// from `ProjectManager::workspace_for`, for a conversation bound to a
/// project. Either way, nothing is checked out here that was not
/// already on disk: that is `ProjectManager`'s own job for the
/// persistent case, and simply not this function's problem for the
/// scratch case, which starts empty.
///
/// Provisions nothing eagerly: constructing either kind of
/// [`WorkspaceSession`] does no podman work at all, so this always
/// succeeds immediately for [`SandboxPolicy::Optional`] and
/// [`SandboxPolicy::Required`] alike (barring a filesystem error) - the
/// tools are present in the schema from the very first moment a turn
/// starts, and a real container is only created the first time one of
/// them is actually called. See `docs/plans/ai/milestone-7-projects.md`,
/// commit 218, for why this resolves gap 9 more completely than the
/// eager-vs-degrade distinction alone could.
pub async fn provision_if_needed(
    policy: SandboxPolicy,
    config: SandboxConfig,
    workspace: Option<PathBuf>,
    base_tools: &Arc<ToolRegistry>,
) -> Result<Option<ProvisionedSandbox>, AiError> {
    if policy == SandboxPolicy::Forbidden {
        return Ok(None);
    }

    let session = match workspace {
        Some(path) => WorkspaceSession::over_workspace(path, config),
        None => WorkspaceSession::new(config)?,
    };
    let tools = Arc::new(base_tools.with_overlay(sandboxed_tools(Arc::clone(&session))));

    Ok(Some(ProvisionedSandbox { tools, session }))
}

fn sandboxed_tools(workspace: Arc<WorkspaceSession>) -> [Arc<dyn Tool>; 6] {
    [
        Arc::new(ReadTool::new(Arc::clone(&workspace))),
        Arc::new(WriteTool::new(Arc::clone(&workspace))),
        Arc::new(EditTool::new(Arc::clone(&workspace))),
        Arc::new(GrepTool::new(Arc::clone(&workspace))),
        Arc::new(GlobTool::new(Arc::clone(&workspace))),
        Arc::new(BashTool::new(workspace)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_forbidden_policy_provisions_nothing() {
        let base = Arc::new(ToolRegistry::new());
        let result = provision_if_needed(
            SandboxPolicy::Forbidden,
            SandboxConfig::default(),
            None,
            &base,
        )
        .await
        .expect("should succeed");
        assert!(result.is_none());
    }

    /// The whole point of commit 218: this must succeed immediately, with
    /// no podman involved at all, regardless of whether `config.image`
    /// could ever actually be provisioned - a broken image only fails the
    /// first sandboxed tool call that actually needs it (see
    /// `crate::sandbox::session`'s own tests for that half).
    #[tokio::test]
    async fn test_optional_and_required_policies_provision_immediately_without_touching_podman() {
        let base = Arc::new(ToolRegistry::new());
        let config = SandboxConfig {
            image: "definitely-does-not-exist-anywhere:latest".to_string(),
            ..Default::default()
        };

        for policy in [SandboxPolicy::Optional, SandboxPolicy::Required] {
            let provisioned = provision_if_needed(policy, config.clone(), None, &base)
                .await
                .expect("should succeed without ever touching podman")
                .expect("should provision something for a non-Forbidden policy");
            for name in ["read", "write", "edit", "grep", "glob", "bash"] {
                assert!(
                    provisioned.tools.get(name).is_some(),
                    "{name} should be present in the schema immediately"
                );
            }
        }
    }

    /// A conversation bound to a project must provision over that
    /// project's own worktree, never a fresh scratch directory - the
    /// whole point of threading `workspace` through this function at
    /// all.
    #[tokio::test]
    async fn test_a_workspace_override_provisions_over_that_exact_directory_instead_of_scratch() {
        let base = Arc::new(ToolRegistry::new());
        let existing = std::env::temp_dir().join(format!(
            "munibot_ai_provision_override_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&existing).unwrap();

        let provisioned = provision_if_needed(
            SandboxPolicy::Optional,
            SandboxConfig::default(),
            Some(existing.clone()),
            &base,
        )
        .await
        .expect("should succeed without ever touching podman")
        .expect("should provision something for a non-Forbidden policy");

        assert_eq!(provisioned.session.workspace_path(), existing.as_path());

        drop(provisioned);
        assert!(
            existing.is_dir(),
            "an overridden workspace is persistent - it must survive the session that used it"
        );
        std::fs::remove_dir_all(&existing).ok();
    }
}
