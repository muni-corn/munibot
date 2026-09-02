//! Why a project or workspace operation failed.

use thiserror::Error;

use crate::project::ProjectId;

/// Why an operation on a project or its workspaces failed.
#[derive(Error, Debug)]
pub enum ProjectError {
    /// No project with this id exists.
    #[error("project {0:?} wasn't found :<")]
    NotFound(ProjectId),
    /// A project's `slug` must be unique; registration tried to reuse one
    /// already taken.
    #[error("a project with slug {0:?} already exists")]
    SlugTaken(String),
    /// A git operation (clone, fetch, worktree management) failed - see
    /// the error's own text for what git actually said.
    #[error("git operation failed :< {0}")]
    Git(String),
    /// Building a project's image failed - see the error's own text for
    /// what podman, devenv, or nix actually said.
    #[error("couldn't build an image for this project :< {0}")]
    ImageBuild(String),
    /// Refused because a pipeline is currently running against this
    /// project - e.g. `remove_project` while a workspace is still in use.
    #[error("can't do that while a pipeline is running against this project")]
    PipelineRunning,
    /// Catch-all for anything else - a filesystem error, a store failure
    /// not worth its own variant yet.
    #[error("project operation failed :< {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_names_the_project_id() {
        let error = ProjectError::NotFound(ProjectId(42));
        assert!(error.to_string().contains("42"));
    }

    #[test]
    fn test_slug_taken_names_the_slug() {
        let error = ProjectError::SlugTaken("cocoa".to_string());
        assert!(error.to_string().contains("cocoa"));
    }
}
