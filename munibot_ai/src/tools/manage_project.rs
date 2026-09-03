use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    tools::{RiskTier, Tool, ToolCtx, ToolOutcome},
    types::{AiError, ToolSchema},
};

/// Plain, tools-local data for one registration attempt - bundled into a
/// struct rather than passed as individual parameters purely to stay
/// under clippy's own argument-count lint; every field is still just a
/// primitive, never a type from another module.
pub struct RegisterProjectArgs<'a> {
    pub munibot_user_id: u64,
    pub slug: &'a str,
    pub display_name: &'a str,
    pub repo_owner: &'a str,
    pub repo_name: &'a str,
    pub clone_url: &'a str,
    pub default_branch: &'a str,
}

/// What the `manage_project` tool needs to register a new project - a
/// narrow, tools-local mirror of `crate::project::ProjectManager::register`,
/// for the same layering reason `crate::tools::MemoryBackend` documents:
/// `ai::tools` sits below `ai::project` in this crate's dependency graph
/// (`project` already depends on `ai::sandbox`, which itself depends on
/// `ai::tools`), so `tools` cannot name that module's types directly.
/// Whatever constructs this tool bridges the two - see
/// `crate::project::ProjectManagerRegistrar`.
#[async_trait]
pub trait ProjectRegistrar: Send + Sync {
    /// Registers a new project, returning a human-readable summary of
    /// the result - its resolved image strategy, and whether an image
    /// actually got built - for the tool to hand back to the model
    /// verbatim.
    async fn register_project(&self, args: RegisterProjectArgs<'_>) -> Result<String, AiError>;
}

#[derive(Deserialize, JsonSchema)]
struct ManageProjectArgs {
    /// A short, url- and filesystem-safe name, unique across every
    /// project.
    slug: String,
    display_name: String,
    repo_owner: String,
    repo_name: String,
    clone_url: String,
    default_branch: String,
}

/// Registers and clones a project when asked.
///
/// `RiskTier::Privileged` - unreachable from public chat by construction,
/// the same way every other tier-4 tool is (see the tool risk tiers table
/// in `docs/plans/ai/overview.md`). An operator's own explicit grant is
/// what makes this reachable at all; nothing here bypasses
/// `ProjectManager::register`, which is the single place that actually
/// clones a repository, detects its image strategy, and builds an image
/// for it.
pub struct ManageProjectTool {
    registrar: Arc<dyn ProjectRegistrar>,
}

impl ManageProjectTool {
    pub fn new(registrar: Arc<dyn ProjectRegistrar>) -> Self {
        Self { registrar }
    }
}

#[async_trait]
impl Tool for ManageProjectTool {
    fn name(&self) -> &str {
        "manage_project"
    }

    fn description(&self) -> &str {
        "Registers a new project: clones its repository, checks out a default workspace, detects \
         which container image strategy applies (a .munibot/Containerfile, a devenv.nix with the \
         right inputs, or munibot's own base image), and builds an image if one is needed."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Privileged
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<ManageProjectArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        let args: ManageProjectArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        match self
            .registrar
            .register_project(RegisterProjectArgs {
                munibot_user_id: ctx.user_id,
                slug: &args.slug,
                display_name: &args.display_name,
                repo_owner: &args.repo_owner,
                repo_name: &args.repo_name,
                clone_url: &args.clone_url,
                default_branch: &args.default_branch,
            })
            .await
        {
            Ok(summary) => ToolOutcome::ok(summary),
            Err(error) => ToolOutcome::err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::tools::{ConversationId, Platform};

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Web,
            granted_tier,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
            delegation_spend: Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    /// Records every call it receives, so these tests can assert the
    /// tool forwarded exactly what it parsed rather than reimplementing
    /// any registration logic itself.
    #[derive(Default)]
    struct FakeRegistrar {
        calls: Mutex<Vec<(u64, String, String)>>,
        result: Mutex<Option<Result<String, String>>>,
    }

    #[async_trait]
    impl ProjectRegistrar for FakeRegistrar {
        async fn register_project(&self, args: RegisterProjectArgs<'_>) -> Result<String, AiError> {
            self.calls.lock().unwrap().push((
                args.munibot_user_id,
                args.slug.to_string(),
                args.clone_url.to_string(),
            ));
            match self.result.lock().unwrap().clone() {
                Some(Ok(summary)) => Ok(summary),
                Some(Err(message)) => Err(AiError::Other(message)),
                None => Ok("registered".to_string()),
            }
        }
    }

    fn args() -> Value {
        json!({
            "slug": "cocoa",
            "display_name": "Cocoa",
            "repo_owner": "muni-corn",
            "repo_name": "cocoa",
            "clone_url": "https://github.com/muni-corn/cocoa.git",
            "default_branch": "main",
        })
    }

    #[test]
    fn test_tool_metadata() {
        let tool = ManageProjectTool::new(Arc::new(FakeRegistrar::default()));
        assert_eq!(tool.name(), "manage_project");
        assert_eq!(tool.tier(), RiskTier::Privileged);
    }

    #[tokio::test]
    async fn test_forwards_the_invoking_users_id_and_parsed_arguments() {
        let registrar = Arc::new(FakeRegistrar::default());
        let tool = ManageProjectTool::new(registrar.clone());

        let outcome = tool.invoke(args(), &ctx(RiskTier::Privileged)).await;
        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");

        let calls = registrar.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            (
                1,
                "cocoa".to_string(),
                "https://github.com/muni-corn/cocoa.git".to_string()
            )
        );
    }

    #[tokio::test]
    async fn test_requires_the_privileged_tier() {
        let tool = ManageProjectTool::new(Arc::new(FakeRegistrar::default()));
        let outcome = tool.invoke(args(), &ctx(RiskTier::Sandbox)).await;
        assert!(matches!(outcome, ToolOutcome::Fatal(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let tool = ManageProjectTool::new(Arc::new(FakeRegistrar::default()));
        let outcome = tool
            .invoke(json!({"slug": 5}), &ctx(RiskTier::Privileged))
            .await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_a_registration_failure_is_a_recoverable_error_not_fatal() {
        let registrar = Arc::new(FakeRegistrar::default());
        *registrar.result.lock().unwrap() =
            Some(Err("a project with that slug already exists".to_string()));
        let tool = ManageProjectTool::new(registrar);

        let outcome = tool.invoke(args(), &ctx(RiskTier::Privileged)).await;
        match outcome {
            ToolOutcome::Err(message) => assert!(message.contains("already exists")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }
}
