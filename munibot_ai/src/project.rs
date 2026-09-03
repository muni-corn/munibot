//! Projects and workspaces: a project is a git repository munibot has
//! cloned, and a workspace is a `git worktree` of one, on its own branch.
//! See `docs/plans/ai/milestone-7-projects.md` for the full design - the
//! filesystem layout, the three image strategies, and why the bare clone
//! matters.
//!
//! Depends on `ai::sandbox` and `munibot_vcs`, and is depended on by
//! `ai::pipeline` - keep it below `ai::pipeline` in the dependency graph in
//! `docs/plans/ai/overview.md`.

mod clone;
mod collect;
mod config;
mod devenv_image;
mod error;
mod image;
mod image_cache;
mod manager;
mod store;
mod strategy;
#[cfg(test)]
pub(crate) mod test_support;
mod tool_backend;
mod types;
mod worktree;

pub use clone::{bare_repo_path, clone_or_fetch_project};
pub use collect::collect_unused_workspaces;
pub use config::ProjectsSettings;
pub use devenv_image::build_devenv_image;
pub use error::ProjectError;
pub use image::build_containerfile_image;
pub use image_cache::{ImageBuilder, compute_source_hash, image_tag};
pub use manager::{ProjectManager, RegisterProjectSpec, WorkspaceOwner};
pub use store::{DieselProjectStore, InMemoryProjectStore, ProjectStore};
pub use strategy::{SkipReason, detect_image_strategy};
pub use tool_backend::ProjectManagerRegistrar;
pub use types::{
    ImageStatus, ImageStrategy, NewProject, NewWorkspace, Project, ProjectId, Workspace,
    WorkspaceId,
};
pub use worktree::{
    WorktreeEntry, create_worktree, existing_branches, list_worktrees, prune_worktrees,
    reconcile_workspaces, remove_worktree,
};
