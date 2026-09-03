//! Pure project and workspace domain types: no git, no database, no
//! filesystem - just the shapes later commits in this module operate on.

use std::{path::PathBuf, str::FromStr};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display, EnumString};

use crate::sandbox::config::SandboxNetworkConfig;

/// A stable identifier for one project, assigned by the project store when
/// the project is registered.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, JsonSchema)]
#[serde(transparent)]
pub struct ProjectId(pub i64);

/// A stable identifier for one workspace, assigned by the project store
/// when the workspace is created.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, JsonSchema)]
#[serde(transparent)]
pub struct WorkspaceId(pub i64);

/// How a project's container image is built, in the order
/// `crate::project`'s own resolution function tries them.
///
/// Serializes and deserializes as its `strum` snake-case string form
/// (`"containerfile"`/`"devenv"`/`"base"`), matching the convention
/// `munibot_core::Permission` already established.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, JsonSchema)]
#[strum(serialize_all = "snake_case")]
pub enum ImageStrategy {
    /// `.munibot/Containerfile` exists in the project's worktree.
    Containerfile,
    /// `devenv.nix` exists, and both `nix2container` and `mk-shell-bin` are
    /// declared inputs.
    Devenv,
    /// No project-specific strategy applies - `[ai.sandbox].image` is used
    /// directly. Always available, tried last.
    Base,
}

impl Serialize for ImageStrategy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ImageStrategy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw)
            .map_err(|_| serde::de::Error::custom(format!("'{raw}' isn't a known image strategy")))
    }
}

/// Whether a project's image has been built yet, and what happened the
/// last time it tried.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImageStatus {
    /// No build has ever been attempted (or attempted since the last
    /// change to whatever the strategy's declared inputs are).
    Unbuilt,
    /// A build succeeded and this tag exists locally.
    Built {
        tag: String,
        /// The first sixteen hex characters of a SHA-256 over the
        /// strategy's declared inputs, plus a constant salt - see
        /// `docs/plans/ai/milestone-7-projects.md`'s image strategy
        /// section. Used to skip a rebuild when nothing relevant changed.
        hash: String,
        at: DateTime<Utc>,
    },
    /// The last build attempt failed. `reason` is a real, actionable
    /// sentence - this is what the projects page shows verbatim, per
    /// `AGENTS.md`'s error-message guidance.
    Failed { reason: String },
}

impl ImageStatus {
    /// A short, stable label for this status - for logging and the
    /// projects page's status column.
    pub fn label(&self) -> &'static str {
        match self {
            ImageStatus::Unbuilt => "unbuilt",
            ImageStatus::Built { .. } => "built",
            ImageStatus::Failed { .. } => "failed",
        }
    }
}

/// A project: a git repository munibot has cloned, one row in
/// `ai_projects`, one directory on disk, one container image.
///
/// Not `JsonSchema`: it embeds `munibot_vcs::RepoRef`/`TriggerMode`, which
/// don't derive it, and nothing schema-facing (a tool argument, a handoff)
/// needs a project embedded directly yet - see `munibot_api`'s own
/// wire-DTO convention (`PipelineSummary` et al.) for what a schema-facing
/// projection of this type should look like when one is needed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    /// A short, URL- and filesystem-safe name, unique across every
    /// project - used in the on-disk path and the image tag
    /// (`munibot-project-<slug>:<hash>`).
    pub slug: String,
    pub display_name: String,
    pub repo: munibot_vcs::RepoRef,
    /// The URL this project was (or will be) cloned from. Never carries a
    /// credential - see `docs/plans/ai/milestone-7-projects.md`'s git
    /// plumbing section for how a token is threaded through instead.
    pub clone_url: String,
    pub default_branch: String,
    /// `<ai.projects.root>/<munibot_user_id>/<slug>/`, the directory
    /// holding this project's bare clone and every workspace's worktree.
    pub local_path: PathBuf,
    pub image_strategy: ImageStrategy,
    pub image_status: ImageStatus,
    /// The network policy every sandbox over one of this project's
    /// workspaces uses. Closed by default, matching
    /// `crate::sandbox::config::SandboxConfig`'s own default.
    pub network_policy: SandboxNetworkConfig,
    /// How this project decides which forge events start a pipeline run.
    /// `None` until an operator (or the projects page) configures one -
    /// an unconfigured project's webhook deliveries are ignored rather
    /// than defaulting to any particular trigger.
    pub trigger: Option<munibot_vcs::TriggerMode>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A workspace: a `git worktree` of a project, on its own branch. One row
/// in `ai_workspaces`, one directory on disk. Persistent - it survives a
/// run, unlike the ephemeral sandbox container over it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub project_id: ProjectId,
    /// Unique within its project - `pipeline-142`, `chat-8891`,
    /// `scratch-a3f1`.
    pub name: String,
    pub branch: String,
    /// `<project.local_path>/worktrees/<name>/`.
    pub path: PathBuf,
    /// The pipeline this workspace belongs to, if any. An opaque foreign
    /// id rather than `crate::pipeline::PipelineId`: `ai::project` sits
    /// below `ai::pipeline` in the dependency graph (see this module's
    /// own doc comment), so it cannot name that module's types.
    pub pipeline_id: Option<i64>,
    /// The conversation this workspace is bound to, if any. Opaque for
    /// the same reason as `pipeline_id`.
    pub conversation_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    /// Updated every time a session actually uses this workspace - what
    /// the workspace-collection sweep (commit 221) compares
    /// `[ai.projects].workspace_ttl` against.
    pub last_used_at: DateTime<Utc>,
}

/// Everything needed to register a new project, before a
/// [`crate::project::ProjectStore`] assigns it an id and timestamps and
/// sets its image status to [`ImageStatus::Unbuilt`].
#[derive(Clone, Debug, PartialEq)]
pub struct NewProject {
    pub slug: String,
    pub display_name: String,
    pub repo: munibot_vcs::RepoRef,
    pub clone_url: String,
    pub default_branch: String,
    pub local_path: PathBuf,
    pub image_strategy: ImageStrategy,
    pub network_policy: SandboxNetworkConfig,
    pub trigger: Option<munibot_vcs::TriggerMode>,
}

/// Everything needed to create a new workspace, before a
/// [`crate::project::ProjectStore`] assigns it an id and timestamps.
#[derive(Clone, Debug, PartialEq)]
pub struct NewWorkspace {
    pub project_id: ProjectId,
    pub name: String,
    pub branch: String,
    pub path: PathBuf,
    pub pipeline_id: Option<i64>,
    pub conversation_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_id_serializes_as_a_bare_number() {
        let encoded = serde_json::to_value(ProjectId(7)).unwrap();
        assert_eq!(encoded, serde_json::json!(7));
    }

    #[test]
    fn test_workspace_id_serializes_as_a_bare_number() {
        let encoded = serde_json::to_value(WorkspaceId(11)).unwrap();
        assert_eq!(encoded, serde_json::json!(11));
    }

    #[test]
    fn test_every_image_strategy_round_trips_through_display_and_from_str() {
        for strategy in [
            ImageStrategy::Containerfile,
            ImageStrategy::Devenv,
            ImageStrategy::Base,
        ] {
            let as_string = strategy.to_string();
            let parsed = ImageStrategy::from_str(&as_string).unwrap();
            assert_eq!(strategy, parsed);
        }
    }

    #[test]
    fn test_image_strategy_serializes_as_a_snake_case_string() {
        assert_eq!(
            serde_json::to_string(&ImageStrategy::Containerfile).unwrap(),
            "\"containerfile\""
        );
        assert_eq!(
            serde_json::to_string(&ImageStrategy::Devenv).unwrap(),
            "\"devenv\""
        );
        assert_eq!(
            serde_json::to_string(&ImageStrategy::Base).unwrap(),
            "\"base\""
        );
    }

    #[test]
    fn test_image_strategy_deserializes_from_its_string_form() {
        let strategy: ImageStrategy = serde_json::from_str("\"devenv\"").unwrap();
        assert_eq!(strategy, ImageStrategy::Devenv);
    }

    #[test]
    fn test_image_strategy_rejects_an_unknown_string() {
        let error = serde_json::from_str::<ImageStrategy>("\"dockerfile\"").unwrap_err();
        assert!(error.to_string().contains("dockerfile"));
    }

    #[test]
    fn test_every_image_status_is_distinguishable_from_every_other() {
        let now = Utc::now();
        let statuses = [
            ImageStatus::Unbuilt,
            ImageStatus::Built {
                tag: "munibot-project-cocoa:abc123".to_string(),
                hash: "abc123".to_string(),
                at: now,
            },
            ImageStatus::Failed {
                reason: "devenv.nix is missing nix2container".to_string(),
            },
        ];
        for (i, a) in statuses.iter().enumerate() {
            for (j, b) in statuses.iter().enumerate() {
                assert_eq!(i == j, a == b, "index {i} vs {j}");
            }
        }
    }

    #[test]
    fn test_image_status_label_is_stable_per_variant() {
        assert_eq!(ImageStatus::Unbuilt.label(), "unbuilt");
        assert_eq!(
            ImageStatus::Built {
                tag: "t".to_string(),
                hash: "h".to_string(),
                at: Utc::now(),
            }
            .label(),
            "built"
        );
        assert_eq!(
            ImageStatus::Failed {
                reason: "r".to_string()
            }
            .label(),
            "failed"
        );
    }

    fn sample_project() -> Project {
        Project {
            id: ProjectId(1),
            slug: "cocoa".to_string(),
            display_name: "Cocoa".to_string(),
            repo: munibot_vcs::RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", "cocoa"),
            clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
            default_branch: "main".to_string(),
            local_path: PathBuf::from("/var/lib/munibot/projects/1/cocoa"),
            image_strategy: ImageStrategy::Base,
            image_status: ImageStatus::Unbuilt,
            network_policy: SandboxNetworkConfig::None,
            trigger: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_project_round_trips_through_json() {
        let project = sample_project();
        let encoded = serde_json::to_string(&project).unwrap();
        let decoded: Project = serde_json::from_str(&encoded).unwrap();
        assert_eq!(project, decoded);
    }

    #[test]
    fn test_workspace_round_trips_through_json() {
        let workspace = Workspace {
            id: WorkspaceId(1),
            project_id: ProjectId(1),
            name: "pipeline-142".to_string(),
            branch: "munibot/issue-17".to_string(),
            path: PathBuf::from("/var/lib/munibot/projects/1/cocoa/worktrees/pipeline-142"),
            pipeline_id: Some(142),
            conversation_id: None,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
        };
        let encoded = serde_json::to_string(&workspace).unwrap();
        let decoded: Workspace = serde_json::from_str(&encoded).unwrap();
        assert_eq!(workspace, decoded);
    }
}
