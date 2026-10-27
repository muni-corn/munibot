//! Git worktree management: creating, listing, and removing the
//! persistent worktrees under a project's bare clone, and reconciling
//! `ai_workspaces` against what is actually on disk after a crash.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::project::{Project, ProjectError, ProjectStore};

/// One worktree `git worktree list` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// `None` for a detached-HEAD worktree - never expected here (every
    /// worktree this module creates checks out a real branch), but
    /// parsed honestly rather than assumed away.
    pub branch: Option<String>,
}

/// Creates a worktree at `worktree_path`, on `branch`.
///
/// If `branch` already exists in the bare repo, the worktree simply
/// checks it out - what a resumed pipeline or a re-entered chat session
/// does, continuing on the exact branch it started on. Otherwise, a new
/// branch is created from `refs/remotes/origin/<base_branch>` (the
/// up-to-date tracking ref a fetch advances - see `crate::project::clone`'s
/// own doc comment for why this is `refs/remotes/origin/*` and not
/// `refs/heads/*`).
pub async fn create_worktree(
    bare_repo: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: &str,
) -> Result<(), ProjectError> {
    let bare_repo_str = bare_repo.display().to_string();
    let worktree_path_str = worktree_path.display().to_string();

    if branch_exists(bare_repo, branch).await? {
        run_git(&[
            "-C",
            &bare_repo_str,
            "worktree",
            "add",
            &worktree_path_str,
            branch,
        ])
        .await
    } else {
        let base_ref = format!("refs/remotes/origin/{base_branch}");
        run_git(&[
            "-C",
            &bare_repo_str,
            "worktree",
            "add",
            "-b",
            branch,
            &worktree_path_str,
            &base_ref,
        ])
        .await
    }
}

/// Whether `branch` already exists as a local branch in the bare repo.
async fn branch_exists(bare_repo: &Path, branch: &str) -> Result<bool, ProjectError> {
    let status = Command::new("git")
        .args([
            "-C",
            &bare_repo.display().to_string(),
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .await
        .map_err(|error| ProjectError::Git(format!("couldn't run git :< {error}")))?;
    Ok(status.success())
}

/// Every local branch already in the bare repo - what a caller passes to
/// `crate::pipeline::resolve_branch_name` as `existing_branches`, so a
/// freshly computed branch name never collides with one already in use.
/// `resolve_branch_name` has been built and tested since milestone 5 with
/// zero callers; this is the seam that finally gives it one.
pub async fn existing_branches(bare_repo: &Path) -> Result<Vec<String>, ProjectError> {
    let output = Command::new("git")
        .args([
            "-C",
            &bare_repo.display().to_string(),
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/",
        ])
        .output()
        .await
        .map_err(|error| ProjectError::Git(format!("couldn't run git :< {error}")))?;

    if !output.status.success() {
        return Err(ProjectError::Git(format!(
            "git for-each-ref failed :< {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Every worktree currently registered against the bare repo, per `git
/// worktree list`'s own administrative records - not a directory listing,
/// so a worktree directory removed without going through git (or `git
/// worktree prune`) still appears here until pruned.
pub async fn list_worktrees(bare_repo: &Path) -> Result<Vec<WorktreeEntry>, ProjectError> {
    let output = Command::new("git")
        .args([
            "-C",
            &bare_repo.display().to_string(),
            "worktree",
            "list",
            "--porcelain",
        ])
        .output()
        .await
        .map_err(|error| ProjectError::Git(format!("couldn't run git :< {error}")))?;

    if !output.status.success() {
        return Err(ProjectError::Git(format!(
            "git worktree list failed :< {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(parse_worktree_list(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parses `git worktree list --porcelain`'s output: blocks of `key value`
/// lines separated by blank lines, one block per worktree. The bare
/// repository's own entry (`bare` instead of a `branch` line) is included
/// by git but excluded here - it is not a worktree this module manages.
fn parse_worktree_list(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut is_bare = false;

    let flush = |path: &mut Option<PathBuf>,
                 branch: &mut Option<String>,
                 is_bare: &mut bool,
                 entries: &mut Vec<WorktreeEntry>| {
        if let Some(path) = path.take()
            && !*is_bare
        {
            entries.push(WorktreeEntry {
                path,
                branch: branch.take(),
            });
        }
        *branch = None;
        *is_bare = false;
    };

    for line in porcelain.lines() {
        if line.is_empty() {
            flush(&mut path, &mut branch, &mut is_bare, &mut entries);
            path = None;
            continue;
        }

        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string(),
            );
        } else if line == "bare" {
            is_bare = true;
        }
    }
    flush(&mut path, &mut branch, &mut is_bare, &mut entries);

    entries
}

/// Removes a worktree, deleting its directory and git's own
/// administrative record for it in one step.
pub async fn remove_worktree(bare_repo: &Path, worktree_path: &Path) -> Result<(), ProjectError> {
    run_git(&[
        "-C",
        &bare_repo.display().to_string(),
        "worktree",
        "remove",
        "--force",
        &worktree_path.display().to_string(),
    ])
    .await
}

/// Removes administrative records for worktrees whose directory is
/// already gone - what a crash between removing a worktree's directory
/// and calling [`remove_worktree`] (or a directory removed by some other
/// means entirely) leaves behind.
pub async fn prune_worktrees(bare_repo: &Path) -> Result<(), ProjectError> {
    run_git(&["-C", &bare_repo.display().to_string(), "worktree", "prune"]).await
}

/// Reconciles `ai_workspaces` against what is actually on disk for one
/// project: prunes git's own stale administrative records first, then
/// removes any workspace row whose worktree directory no longer exists.
///
/// Run after a crash (or at startup) to clean up after an interruption
/// partway through removing a workspace - see
/// `docs/plans/ai/milestone-7-projects.md`'s ordering note on why the
/// workspace-collection sweep (a later commit) removes the worktree, then
/// prunes, then deletes the row, in that exact order: this function is
/// what makes an interruption at any point in that sequence self-healing
/// rather than an orphan.
pub async fn reconcile_workspaces(
    store: &dyn ProjectStore,
    project: &Project,
) -> Result<(), ProjectError> {
    let bare_repo = crate::project::bare_repo_path(&project.local_path);
    if bare_repo.is_dir() {
        prune_worktrees(&bare_repo).await?;
    }

    for workspace in store.list_workspaces_for_project(project.id).await? {
        if !workspace.path.is_dir() {
            store.delete_workspace(workspace.id).await?;
        }
    }
    Ok(())
}

/// Runs `git` with `args`, failing with the process's own stderr on a
/// nonzero exit.
async fn run_git(args: &[&str]) -> Result<(), ProjectError> {
    let output = Command::new("git")
        .args(args)
        .output()
        .await
        .map_err(|error| ProjectError::Git(format!("couldn't run git :< {error}")))?;

    if !output.status.success() {
        return Err(ProjectError::Git(format!(
            "git {} failed :< {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        project::{ImageStrategy, InMemoryProjectStore, NewProject, NewWorkspace},
        sandbox::config::SandboxNetworkConfig,
    };

    /// A local bare repository with one commit on `main`, real git
    /// plumbing, zero network - matching `crate::project::clone`'s own
    /// testing strategy.
    struct TestRepo {
        project_root: PathBuf,
        bare_repo: PathBuf,
    }

    impl TestRepo {
        async fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let upstream_bare =
                std::env::temp_dir().join(format!("munibot_ai_worktree_upstream_{unique}"));
            let work_dir = std::env::temp_dir().join(format!("munibot_ai_worktree_work_{unique}"));
            std::fs::create_dir_all(&work_dir).unwrap();

            run_git(&[
                "init",
                "--bare",
                "--initial-branch",
                "main",
                &upstream_bare.display().to_string(),
            ])
            .await
            .unwrap();

            let work_dir_str = work_dir.display().to_string();
            run_git(&["-C", &work_dir_str, "init", "--initial-branch", "main"])
                .await
                .unwrap();
            run_git(&[
                "-C",
                &work_dir_str,
                "config",
                "user.email",
                "muni@example.com",
            ])
            .await
            .unwrap();
            run_git(&["-C", &work_dir_str, "config", "user.name", "muni"])
                .await
                .unwrap();
            run_git(&["-C", &work_dir_str, "config", "commit.gpgsign", "false"])
                .await
                .unwrap();
            std::fs::write(work_dir.join("README.md"), "hello").unwrap();
            run_git(&["-C", &work_dir_str, "add", "."]).await.unwrap();
            run_git(&["-C", &work_dir_str, "commit", "-m", "initial commit"])
                .await
                .unwrap();
            run_git(&[
                "-C",
                &work_dir_str,
                "push",
                &upstream_bare.display().to_string(),
                "main",
            ])
            .await
            .unwrap();
            std::fs::remove_dir_all(&work_dir).ok();

            // clone_or_fetch_project sets up the refs/remotes/origin/*
            // refspec this module's create_worktree relies on
            let project_root =
                std::env::temp_dir().join(format!("munibot_ai_worktree_project_{unique}"));
            crate::project::clone_or_fetch_project(
                &project_root,
                &format!("file://{}", upstream_bare.display()),
                None,
            )
            .await
            .unwrap();
            std::fs::remove_dir_all(&upstream_bare).ok();

            let bare_repo = crate::project::bare_repo_path(&project_root);
            Self {
                project_root,
                bare_repo,
            }
        }

        fn worktree_path(&self, name: &str) -> PathBuf {
            self.project_root.join("worktrees").join(name)
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.project_root).ok();
        }
    }

    #[tokio::test]
    async fn test_create_worktree_on_a_new_branch_checks_out_the_base_branches_files() {
        let repo = TestRepo::new().await;
        let path = repo.worktree_path("scratch");

        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .expect("should create");

        assert_eq!(
            std::fs::read_to_string(path.join("README.md")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn test_create_worktree_reuses_an_existing_branch() {
        let repo = TestRepo::new().await;
        let first_path = repo.worktree_path("first");
        create_worktree(&repo.bare_repo, &first_path, "munibot/reuse-me", "main")
            .await
            .unwrap();
        remove_worktree(&repo.bare_repo, &first_path).await.unwrap();

        // the branch survives even though the first worktree using it
        // was removed - a second worktree on the same branch must reuse
        // it rather than erroring on "branch already exists"
        let second_path = repo.worktree_path("second");
        create_worktree(&repo.bare_repo, &second_path, "munibot/reuse-me", "main")
            .await
            .expect("should reuse the existing branch");

        assert!(second_path.join("README.md").is_file());
    }

    #[tokio::test]
    async fn test_existing_branches_output_is_exactly_what_resolve_branch_name_expects() {
        let repo = TestRepo::new().await;

        // a branch already occupies the name a fresh pipeline run for
        // this exact issue/title would otherwise pick
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("taken"),
            "munibot/17-fix-the-crash",
            "main",
        )
        .await
        .unwrap();

        let existing = existing_branches(&repo.bare_repo).await.unwrap();

        // crate::pipeline::resolve_branch_name(17, "fix the crash", None,
        // &existing) is the actual caller (a later commit, once a pipeline
        // creates its own workspace); reproducing its collision-suffix logic
        // inline here keeps this module from depending on ai::pipeline,
        // which sits above it in the dependency graph (see this module's
        // own doc comment)
        let base = "munibot/17-fix-the-crash";
        assert!(existing.iter().any(|branch| branch == base));
        let next_free = (2..)
            .map(|attempt| format!("{base}-{attempt}"))
            .find(|candidate| !existing.contains(candidate))
            .unwrap();
        assert_eq!(next_free, "munibot/17-fix-the-crash-2");

        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("fresh"),
            &next_free,
            "main",
        )
        .await
        .expect("the resolved name should be free to use");
    }

    #[tokio::test]
    async fn test_existing_branches_lists_every_local_branch() {
        let repo = TestRepo::new().await;
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("a"),
            "munibot/a",
            "main",
        )
        .await
        .unwrap();
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("b"),
            "munibot/b",
            "main",
        )
        .await
        .unwrap();

        let mut branches = existing_branches(&repo.bare_repo).await.unwrap();
        branches.sort();
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"munibot/a".to_string()));
        assert!(branches.contains(&"munibot/b".to_string()));
    }

    #[tokio::test]
    async fn test_list_worktrees_reports_every_created_worktree_and_its_branch() {
        let repo = TestRepo::new().await;
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("a"),
            "munibot/a",
            "main",
        )
        .await
        .unwrap();
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("b"),
            "munibot/b",
            "main",
        )
        .await
        .unwrap();

        let mut entries = list_worktrees(&repo.bare_repo).await.unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, repo.worktree_path("a"));
        assert_eq!(entries[0].branch, Some("munibot/a".to_string()));
        assert_eq!(entries[1].path, repo.worktree_path("b"));
        assert_eq!(entries[1].branch, Some("munibot/b".to_string()));
    }

    #[tokio::test]
    async fn test_list_worktrees_excludes_the_bare_repository_itself() {
        let repo = TestRepo::new().await;
        create_worktree(
            &repo.bare_repo,
            &repo.worktree_path("a"),
            "munibot/a",
            "main",
        )
        .await
        .unwrap();

        let entries = list_worktrees(&repo.bare_repo).await.unwrap();
        assert_eq!(
            entries.len(),
            1,
            "the bare repo's own entry must not be listed"
        );
    }

    #[tokio::test]
    async fn test_remove_worktree_deletes_the_directory_and_the_administrative_record() {
        let repo = TestRepo::new().await;
        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();

        remove_worktree(&repo.bare_repo, &path)
            .await
            .expect("should remove");

        assert!(!path.exists());
        assert!(list_worktrees(&repo.bare_repo).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_prune_worktrees_cleans_up_a_directory_removed_without_git() {
        let repo = TestRepo::new().await;
        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();

        // simulate a crash: the directory is gone, but git doesn't know yet
        std::fs::remove_dir_all(&path).unwrap();
        assert_eq!(
            list_worktrees(&repo.bare_repo).await.unwrap().len(),
            1,
            "git still thinks the worktree exists until pruned"
        );

        prune_worktrees(&repo.bare_repo)
            .await
            .expect("should prune");

        assert!(list_worktrees(&repo.bare_repo).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_reconcile_workspaces_removes_rows_whose_worktree_directory_is_gone() {
        let repo = TestRepo::new().await;
        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();

        let store = InMemoryProjectStore::new();
        let project = store
            .upsert_project(&NewProject {
                slug: "cocoa".to_string(),
                display_name: "Cocoa".to_string(),
                repo: munibot_vcs::RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", "cocoa"),
                clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
                default_branch: "main".to_string(),
                local_path: repo.project_root.clone(),
                image_strategy: ImageStrategy::Base,
                network_policy: SandboxNetworkConfig::None,
                trigger: None,
                installation_id: None,
            })
            .await
            .unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: path.clone(),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();

        // crash: the worktree directory is gone, but the row is not
        std::fs::remove_dir_all(&path).unwrap();

        reconcile_workspaces(&store, &project)
            .await
            .expect("should reconcile");

        let error = store.get_workspace(workspace.id).await.unwrap_err();
        assert!(matches!(error, ProjectError::Other(_)));
    }

    #[tokio::test]
    async fn test_reconcile_workspaces_leaves_rows_whose_worktree_still_exists() {
        let repo = TestRepo::new().await;
        let path = repo.worktree_path("scratch");
        create_worktree(&repo.bare_repo, &path, "munibot/scratch", "main")
            .await
            .unwrap();

        let store = InMemoryProjectStore::new();
        let project = store
            .upsert_project(&NewProject {
                slug: "cocoa".to_string(),
                display_name: "Cocoa".to_string(),
                repo: munibot_vcs::RepoRef::new(munibot_vcs::Forge::GitHub, "muni-corn", "cocoa"),
                clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
                default_branch: "main".to_string(),
                local_path: repo.project_root.clone(),
                image_strategy: ImageStrategy::Base,
                network_policy: SandboxNetworkConfig::None,
                trigger: None,
                installation_id: None,
            })
            .await
            .unwrap();
        let workspace = store
            .create_workspace(&NewWorkspace {
                project_id: project.id,
                name: "scratch".to_string(),
                branch: "munibot/scratch".to_string(),
                path: path.clone(),
                pipeline_id: None,
                conversation_id: None,
            })
            .await
            .unwrap();

        reconcile_workspaces(&store, &project)
            .await
            .expect("should reconcile");

        assert!(store.get_workspace(workspace.id).await.is_ok());
    }
}
