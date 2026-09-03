//! Provisions a sandbox for one turn, according to a persona's
//! [`SandboxPolicy`].

use std::sync::Arc;

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

/// Adds the six sandbox tools, bound to a fresh [`WorkspaceSession`], onto
/// `base_tools`, unless `policy` is [`SandboxPolicy::Forbidden`].
///
/// Provisions nothing eagerly: a [`WorkspaceSession`] only reserves a
/// scratch workspace directory at construction, so this always succeeds
/// immediately for [`SandboxPolicy::Optional`] and
/// [`SandboxPolicy::Required`] alike (barring a filesystem error) - the
/// tools are present in the schema from the very first moment a turn
/// starts, and a real container is only created the first time one of
/// them is actually called. See `docs/plans/ai/milestone-7-projects.md`,
/// commit 218, for why this resolves gap 9 more completely than the
/// eager-vs-degrade distinction alone could.
///
/// Nothing is checked out into the workspace here - that is
/// [`crate::sandbox::container::Sandbox::checkout`]'s job, and needs a
/// repository to check out that this function has no source for yet (a
/// chat-delegated turn has no issue or pull request behind it). A
/// sandbox provisioned this way starts with an empty workspace a persona
/// can still `write`/`bash` its way through.
pub async fn provision_if_needed(
    policy: SandboxPolicy,
    config: SandboxConfig,
    base_tools: &Arc<ToolRegistry>,
) -> Result<Option<ProvisionedSandbox>, AiError> {
    if policy == SandboxPolicy::Forbidden {
        return Ok(None);
    }

    let session = WorkspaceSession::new(config)?;
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
        let result = provision_if_needed(SandboxPolicy::Forbidden, SandboxConfig::default(), &base)
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
            let provisioned = provision_if_needed(policy, config.clone(), &base)
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
}
