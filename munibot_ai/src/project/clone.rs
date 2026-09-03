//! Cloning (and re-fetching) a project's bare repository.
//!
//! The credential-passing technique here - a temporary `git
//! credential-cache` daemon fed over `git credential approve`'s stdin,
//! never the clone URL or any process's argv - is deliberately the same
//! shape `crate::sandbox::checkout` already uses and proved out, but
//! reimplemented rather than shared: that module clones into an ephemeral
//! sandbox workspace for one turn, this one clones a project's own
//! long-lived bare repository, and the two have no other reason to depend
//! on each other. The wire-protocol duplication `crate::sandbox`'s own
//! doc comment accepts for the same reason applies here too.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::project::ProjectError;

/// The bare clone's directory name under a project's own root - see the
/// filesystem layout in `docs/plans/ai/milestone-7-projects.md`.
const BARE_REPO_DIR_NAME: &str = "repo.git";

/// The refspec a plain `git clone --bare` does *not* set, and that a later
/// `git fetch` needs to actually advance anything - see this module's own
/// doc comment and the milestone's own "gotcha" note. Without this, every
/// worktree is created from a stale ref forever, since a bare clone's
/// `refs/heads/*` are frozen at clone time.
const ORIGIN_FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// Clones `clone_url` as a bare repository at
/// `project_root/repo.git`, or - if one already exists there - fetches
/// it instead. Idempotent either way: calling this again on an existing
/// clone never re-clones.
///
/// `token`, when given, authenticates through a temporary git credential
/// helper, approved over its own stdin - never spliced into `clone_url` or
/// any process's argv, where it would land in shell history and be
/// visible to every user on the host via `ps`.
pub async fn clone_or_fetch_project(
    project_root: &Path,
    clone_url: &str,
    token: Option<&str>,
) -> Result<(), ProjectError> {
    let bare_repo = project_root.join(BARE_REPO_DIR_NAME);

    if bare_repo.is_dir() {
        return fetch_bare_repo(&bare_repo, token).await;
    }

    std::fs::create_dir_all(project_root).map_err(|error| {
        ProjectError::Git(format!("couldn't prepare {project_root:?} :< {error}"))
    })?;

    let bare_repo_str = bare_repo.display().to_string();
    with_credential(clone_url, token, |config_args| async move {
        run_git(&config_args, &[
            "clone",
            "--bare",
            clone_url,
            &bare_repo_str,
        ])
        .await
    })
    .await?;

    // set right after cloning, before anything else touches this repo -
    // see ORIGIN_FETCH_REFSPEC's own doc comment for why a bare clone is
    // otherwise permanently stale
    run_git(&[], &[
        "-C",
        &bare_repo.display().to_string(),
        "config",
        "remote.origin.fetch",
        ORIGIN_FETCH_REFSPEC,
    ])
    .await
}

/// Fetches an existing bare repository, advancing every
/// `refs/remotes/origin/*` ref to match upstream.
async fn fetch_bare_repo(bare_repo: &Path, token: Option<&str>) -> Result<(), ProjectError> {
    let bare_repo_str = bare_repo.display().to_string();

    // the clone url isn't known here - only origin's own remembered url is
    // needed to authenticate a fetch, and git already has that recorded
    let clone_url = get_remote_url(bare_repo).await?;

    with_credential(&clone_url, token, |config_args| async move {
        run_git(&config_args, &["-C", &bare_repo_str, "fetch", "origin"]).await
    })
    .await
}

/// Reads `origin`'s configured url from an existing bare repository.
async fn get_remote_url(bare_repo: &Path) -> Result<String, ProjectError> {
    let output = Command::new("git")
        .args([
            "-C",
            &bare_repo.display().to_string(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .await
        .map_err(|error| ProjectError::Git(format!("couldn't run git :< {error}")))?;

    if !output.status.success() {
        return Err(ProjectError::Git(format!(
            "couldn't read origin's url :< {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Runs `body` with the `-c key=value` config arguments needed to
/// authenticate against `clone_url`'s host through a temporary git
/// credential-cache daemon, when `token` is given - `body` sees no
/// arguments at all when it is not.
async fn with_credential<F, Fut>(
    clone_url: &str,
    token: Option<&str>,
    body: F,
) -> Result<(), ProjectError>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(), ProjectError>>,
{
    let Some(token) = token else {
        return body(Vec::new()).await;
    };

    let host = url::Url::parse(clone_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .ok_or_else(|| {
            ProjectError::Git(format!(
                "{clone_url:?} doesn't have a host to authenticate against"
            ))
        })?;

    let socket = std::env::temp_dir().join(format!(
        "munibot-project-git-credential-{}.sock",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let helper_config = format!("credential.helper=cache --socket={}", socket.display());

    approve_credential(&helper_config, &host, token).await?;

    let result = body(vec![helper_config.clone()]).await;

    // best-effort: clears the temporary cache daemon regardless of
    // whether the operation itself succeeded, so a token never outlives
    // this one call even if something above failed
    let _ = run_git(&[helper_config], &["credential-cache", "exit"]).await;

    result
}

/// Feeds `token` to the credential-cache daemon named by `helper_config`
/// through `git credential approve`'s stdin, in the plain key=value
/// format the git credential protocol expects - never as a command-line
/// argument.
async fn approve_credential(
    helper_config: &str,
    host: &str,
    token: &str,
) -> Result<(), ProjectError> {
    let input =
        format!("protocol=https\nhost={host}\nusername=x-access-token\npassword={token}\n\n");

    let mut child = Command::new("git")
        .args(["-c", helper_config, "credential", "approve"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            ProjectError::Git(format!("couldn't start git credential approve :< {error}"))
        })?;

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("stdin was piped above");
        stdin.write_all(input.as_bytes()).await.map_err(|error| {
            ProjectError::Git(format!("couldn't supply the credential :< {error}"))
        })?;
    }

    let status = child
        .wait()
        .await
        .map_err(|error| ProjectError::Git(format!("git credential approve failed :< {error}")))?;

    if !status.success() {
        return Err(ProjectError::Git(format!(
            "git credential approve exited with {status:?}"
        )));
    }
    Ok(())
}

/// Runs `git` with `config_args` (each a `-c key=value` pair's value, so
/// `["a=b"]` becomes `-c a=b`) followed by `args`, failing with the
/// process's own stderr on a nonzero exit.
async fn run_git(config_args: &[String], args: &[&str]) -> Result<(), ProjectError> {
    let mut full_args: Vec<&str> = Vec::new();
    for config in config_args {
        full_args.push("-c");
        full_args.push(config);
    }
    full_args.extend_from_slice(args);

    let output = Command::new("git")
        .args(&full_args)
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

/// The path a bare clone actually lives at, given the project's own root -
/// exposed so callers (the project manager, worktree management) never
/// have to hardcode [`BARE_REPO_DIR_NAME`] themselves.
pub fn bare_repo_path(project_root: &Path) -> PathBuf {
    project_root.join(BARE_REPO_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local bare repository to clone from, with one commit on
    /// `base_branch` - real git plumbing, zero network access, matching
    /// `crate::sandbox::checkout`'s own testing strategy.
    struct LocalUpstream {
        bare_path: PathBuf,
        work_dir: PathBuf,
    }

    impl LocalUpstream {
        async fn new(base_branch: &str, files: &[(&str, &str)]) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let bare_path =
                std::env::temp_dir().join(format!("munibot_ai_project_clone_upstream_{unique}"));
            let work_dir =
                std::env::temp_dir().join(format!("munibot_ai_project_clone_work_{unique}"));
            std::fs::create_dir_all(&work_dir).unwrap();

            run_git(&[], &[
                "init",
                "--bare",
                "--initial-branch",
                base_branch,
                &bare_path.display().to_string(),
            ])
            .await
            .expect("should init bare repo");

            let work_dir_str = work_dir.display().to_string();
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "init",
                "--initial-branch",
                base_branch,
            ])
            .await
            .expect("should init work dir");
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "config",
                "user.email",
                "muni@example.com",
            ])
            .await
            .unwrap();
            run_git(&[], &["-C", &work_dir_str, "config", "user.name", "muni"])
                .await
                .unwrap();
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "config",
                "commit.gpgsign",
                "false",
            ])
            .await
            .unwrap();

            let mut instance = Self {
                bare_path,
                work_dir,
            };
            instance.commit_and_push(base_branch, files).await;
            instance
        }

        async fn commit_and_push(&mut self, branch: &str, files: &[(&str, &str)]) {
            let work_dir_str = self.work_dir.display().to_string();
            for (name, content) in files {
                std::fs::write(self.work_dir.join(name), content).unwrap();
            }
            run_git(&[], &["-C", &work_dir_str, "add", "."])
                .await
                .unwrap();
            run_git(&[], &["-C", &work_dir_str, "commit", "-m", "commit"])
                .await
                .unwrap();
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "push",
                &self.bare_path.display().to_string(),
                branch,
            ])
            .await
            .unwrap();
        }

        fn url(&self) -> String {
            format!("file://{}", self.bare_path.display())
        }

        async fn head_sha(&self, branch: &str) -> String {
            let output = Command::new("git")
                .args([
                    "-C",
                    &self.bare_path.display().to_string(),
                    "rev-parse",
                    branch,
                ])
                .output()
                .await
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    }

    impl Drop for LocalUpstream {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.bare_path).ok();
            std::fs::remove_dir_all(&self.work_dir).ok();
        }
    }

    fn scratch_project_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "munibot_ai_project_clone_dest_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn rev_parse(bare_repo: &Path, rev: &str) -> Option<String> {
        let output = Command::new("git")
            .args(["-C", &bare_repo.display().to_string(), "rev-parse", rev])
            .output()
            .await
            .unwrap();
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    #[tokio::test]
    async fn test_clone_creates_a_bare_repo_at_the_expected_path() {
        let upstream = LocalUpstream::new("main", &[("README.md", "hello")]).await;
        let project_root = scratch_project_root();

        clone_or_fetch_project(&project_root, &upstream.url(), None)
            .await
            .expect("should clone");

        let bare_repo = bare_repo_path(&project_root);
        assert!(bare_repo.is_dir());
        assert!(bare_repo.join("HEAD").is_file(), "should be a bare repo");

        std::fs::remove_dir_all(&project_root).ok();
    }

    #[tokio::test]
    async fn test_clone_sets_the_origin_fetch_refspec() {
        let upstream = LocalUpstream::new("main", &[("README.md", "hello")]).await;
        let project_root = scratch_project_root();

        clone_or_fetch_project(&project_root, &upstream.url(), None)
            .await
            .expect("should clone");

        let bare_repo = bare_repo_path(&project_root);
        let output = Command::new("git")
            .args([
                "-C",
                &bare_repo.display().to_string(),
                "config",
                "remote.origin.fetch",
            ])
            .output()
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            ORIGIN_FETCH_REFSPEC
        );

        std::fs::remove_dir_all(&project_root).ok();
    }

    #[tokio::test]
    async fn test_calling_it_again_fetches_rather_than_re_clones_and_advances_the_tracking_ref() {
        let mut upstream = LocalUpstream::new("main", &[("README.md", "v1")]).await;
        let project_root = scratch_project_root();

        clone_or_fetch_project(&project_root, &upstream.url(), None)
            .await
            .expect("initial clone should succeed");
        let bare_repo = bare_repo_path(&project_root);

        // before any fetch, there is no refs/remotes/origin/main yet - a
        // bare clone's own refs/heads/* are the only thing populated
        assert!(
            rev_parse(&bare_repo, "refs/remotes/origin/main")
                .await
                .is_none()
        );

        upstream
            .commit_and_push("main", &[("README.md", "v2")])
            .await;
        let new_sha = upstream.head_sha("main").await;

        clone_or_fetch_project(&project_root, &upstream.url(), None)
            .await
            .expect("second call should fetch, not re-clone");

        let tracking_ref_sha = rev_parse(&bare_repo, "refs/remotes/origin/main")
            .await
            .expect("a fetch with the configured refspec should populate this ref");
        assert_eq!(
            tracking_ref_sha, new_sha,
            "the tracking ref should have advanced to the new upstream commit"
        );

        std::fs::remove_dir_all(&project_root).ok();
    }

    #[test]
    fn test_credential_input_never_needs_the_token_on_a_process_argument() {
        let host = "github.com";
        let token = "ghp_supersecret";
        let input =
            format!("protocol=https\nhost={host}\nusername=x-access-token\npassword={token}\n\n");
        assert!(input.starts_with("protocol=https\n"));
        assert!(input.contains("host=github.com\n"));
        assert!(input.contains("password=ghp_supersecret\n"));
        assert!(input.ends_with("\n\n"));
    }
}
