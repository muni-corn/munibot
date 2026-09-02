//! Projects and workspaces: a project is a git repository munibot has
//! cloned, and a workspace is a `git worktree` of one, on its own branch.
//! See `docs/plans/ai/milestone-7-projects.md` for the full design - the
//! filesystem layout, the three image strategies, and why the bare clone
//! matters.
//!
//! Depends on `ai::sandbox` and `munibot_vcs`, and is depended on by
//! `ai::pipeline` - keep it below `ai::pipeline` in the dependency graph in
//! `docs/plans/ai/overview.md`.

mod error;
mod types;

pub use error::ProjectError;
pub use types::{ImageStatus, ImageStrategy, Project, ProjectId, Workspace, WorkspaceId};
