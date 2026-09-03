//! Projects and workspaces: a project is a git repository munibot has
//! cloned, and a workspace is a `git worktree` of one, on its own branch.
//! See `docs/plans/ai/milestone-7-projects.md` for the full design - the
//! filesystem layout, the three image strategies, and why the bare clone
//! matters.
//!
//! Depends on `ai::sandbox` and `munibot_vcs`, and is depended on by
//! `ai::pipeline` - keep it below `ai::pipeline` in the dependency graph in
//! `docs/plans/ai/overview.md`.

mod config;
mod error;
mod store;
mod types;

pub use config::ProjectsSettings;
pub use error::ProjectError;
pub use store::{DieselProjectStore, InMemoryProjectStore, ProjectStore};
pub use types::{
    ImageStatus, ImageStrategy, NewProject, NewWorkspace, Project, ProjectId, Workspace,
    WorkspaceId,
};
