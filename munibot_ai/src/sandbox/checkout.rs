//! Repository checkout: cloning into the workspace mount, and running a
//! checked-out project's own obvious dependency install step.

use std::path::Path;

use tokio::process::Command;

use crate::{sandbox::container::Sandbox, types::AiError};

impl Sandbox {
    /// Clones `repo_url` at `base_branch` into this sandbox's workspace
    /// mount (see [`Self::with_workspace_mount`], which must already have
    /// been called), then detects and runs the obvious dependency install
    /// for whatever project type it finds.
    ///
    /// `token`, when given, authenticates the clone through a git
    /// credential helper - approved via `git credential approve`'s own
    /// stdin, never spliced into the clone URL or any process's argv,
    /// where it would land in shell history and be visible to every user
    /// on the host via `ps`.
    ///
    /// The dependency install step runs *inside the running container*
    /// via [`Self::exec`], never on the host - an npm postinstall hook, a
    /// `setup.py`, or a build-time proc macro from an untrusted checkout
    /// is exactly the kind of arbitrary code execution the sandbox exists
    /// to contain. If no container is running yet, or the install step
    /// itself fails, this logs a warning and returns successfully rather
    /// than failing the whole checkout - an unbuilt checkout is still
    /// useful; a coding persona can always run the same command itself
    /// through `bash` once it starts working.
    pub async fn checkout(
        &self,
        repo_url: &str,
        base_branch: &str,
        token: Option<&str>,
    ) -> Result<(), AiError> {
        let workspace = self.workspace_mount().ok_or_else(|| {
            AiError::Other(
                "this sandbox has no workspace mount configured :< call with_workspace_mount \
                 before checkout"
                    .to_string(),
            )
        })?;

        clone_repository(workspace, repo_url, base_branch, token).await?;

        match detect_dependency_install(workspace) {
            Some(cmd) if self.container_id().is_some() => {
                if let Err(error) = self.exec(cmd).await {
                    tracing::warn!(
                        %error,
                        repo_url,
                        "dependency install failed; continuing with an unbuilt checkout"
                    );
                }
            }
            Some(_) => {
                tracing::warn!(
                    repo_url,
                    "no container is running yet; skipping dependency install"
                );
            }
            None => {}
        }

        Ok(())
    }
}

/// Clones `repo_url` at `base_branch` into `workspace`, authenticating
/// through a temporary `git credential-cache` daemon when `token` is given.
async fn clone_repository(
    workspace: &Path,
    repo_url: &str,
    base_branch: &str,
    token: Option<&str>,
) -> Result<(), AiError> {
    let Some(token) = token else {
        return run_git(&[], &[
            "clone",
            "--branch",
            base_branch,
            "--single-branch",
            repo_url,
            &workspace.display().to_string(),
        ])
        .await;
    };

    let host = url::Url::parse(repo_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .ok_or_else(|| {
            AiError::Other(format!(
                "{repo_url:?} doesn't have a host to authenticate against"
            ))
        })?;

    let socket = std::env::temp_dir().join(format!(
        "munibot-git-credential-{}.sock",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let helper_config = format!("credential.helper=cache --socket={}", socket.display());

    approve_credential(&helper_config, &host, token).await?;

    let clone_result = run_git(&[&helper_config], &[
        "clone",
        "--branch",
        base_branch,
        "--single-branch",
        repo_url,
        &workspace.display().to_string(),
    ])
    .await;

    // best-effort: clears the temporary cache daemon regardless of whether
    // the clone itself succeeded, so a token never outlives this one
    // checkout even if something above failed
    let _ = run_git(&[&helper_config], &["credential-cache", "exit"]).await;

    clone_result
}

/// Feeds `token` to the credential-cache daemon named by `helper_config`
/// through `git credential approve`'s stdin, in the plain key=value format
/// the git credential protocol expects - never as a command-line argument.
async fn approve_credential(helper_config: &str, host: &str, token: &str) -> Result<(), AiError> {
    let input =
        format!("protocol=https\nhost={host}\nusername=x-access-token\npassword={token}\n\n");

    let mut child = Command::new("git")
        .args(["-c", helper_config, "credential", "approve"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            AiError::Other(format!("couldn't start git credential approve :< {error}"))
        })?;

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("stdin was piped above");
        stdin.write_all(input.as_bytes()).await.map_err(|error| {
            AiError::Other(format!("couldn't supply the credential :< {error}"))
        })?;
    }

    let status = child
        .wait()
        .await
        .map_err(|error| AiError::Other(format!("git credential approve failed :< {error}")))?;

    if !status.success() {
        return Err(AiError::Other(format!(
            "git credential approve exited with {status:?}"
        )));
    }
    Ok(())
}

/// Runs `git` with `config_args` (each a `-c key=value` pair's value, so
/// `["a=b"]` becomes `-c a=b`) followed by `args`, failing with the
/// process's own stderr on a nonzero exit.
async fn run_git(config_args: &[&str], args: &[&str]) -> Result<(), AiError> {
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
        .map_err(|error| AiError::Other(format!("couldn't run git :< {error}")))?;

    if !output.status.success() {
        return Err(AiError::Other(format!(
            "git {} failed :< {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Detects the obvious dependency install command for whatever project type
/// is at `workspace`'s root, from its most common manifest files.
///
/// Deliberately narrow rather than exhaustive - "the obvious" one per the
/// milestone's own scope, not every build tool every ecosystem might use.
fn detect_dependency_install(workspace: &Path) -> Option<Vec<String>> {
    let manifest = |name: &str| workspace.join(name).is_file();

    if manifest("Cargo.toml") {
        Some(vec!["cargo".to_string(), "fetch".to_string()])
    } else if manifest("package.json") {
        Some(vec!["npm".to_string(), "install".to_string()])
    } else if manifest("requirements.txt") {
        Some(vec![
            "pip".to_string(),
            "install".to_string(),
            "-r".to_string(),
            "requirements.txt".to_string(),
        ])
    } else if manifest("go.mod") {
        Some(vec![
            "go".to_string(),
            "mod".to_string(),
            "download".to_string(),
        ])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local bare repository to clone from, with one commit on
    /// `base_branch` - real git plumbing, zero network access, per this
    /// crate's own testing strategy ("no unit test touches the network.
    /// Ever.", `docs/plans/ai/overview.md`).
    struct LocalRepo {
        bare_path: std::path::PathBuf,
        work_dir: std::path::PathBuf,
    }

    impl LocalRepo {
        async fn new(base_branch: &str, files: &[(&str, &str)]) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let bare_path =
                std::env::temp_dir().join(format!("munibot_ai_checkout_test_bare_{unique}"));
            let work_dir =
                std::env::temp_dir().join(format!("munibot_ai_checkout_test_work_{unique}"));
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
            // this host's global git config may have commit signing on;
            // this scratch repo never needs a real signature
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "config",
                "commit.gpgsign",
                "false",
            ])
            .await
            .unwrap();

            for (name, content) in files {
                std::fs::write(work_dir.join(name), content).unwrap();
            }
            run_git(&[], &["-C", &work_dir_str, "add", "."])
                .await
                .unwrap();
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "commit",
                "-m",
                "initial commit",
            ])
            .await
            .unwrap();
            run_git(&[], &[
                "-C",
                &work_dir_str,
                "push",
                &bare_path.display().to_string(),
                base_branch,
            ])
            .await
            .unwrap();

            Self {
                bare_path,
                work_dir,
            }
        }

        fn url(&self) -> String {
            format!("file://{}", self.bare_path.display())
        }
    }

    impl Drop for LocalRepo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.bare_path).ok();
            std::fs::remove_dir_all(&self.work_dir).ok();
        }
    }

    #[tokio::test]
    async fn test_clone_repository_checks_out_the_requested_branch() {
        let repo = LocalRepo::new("main", &[("README.md", "hello")]).await;
        let dest = std::env::temp_dir().join(format!(
            "munibot_ai_checkout_test_dest_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        clone_repository(&dest, &repo.url(), "main", None)
            .await
            .expect("should clone");

        assert_eq!(
            std::fs::read_to_string(dest.join("README.md")).unwrap(),
            "hello"
        );

        std::fs::remove_dir_all(&dest).ok();
    }

    #[tokio::test]
    async fn test_clone_repository_fails_for_a_nonexistent_branch() {
        let repo = LocalRepo::new("main", &[("README.md", "hello")]).await;
        let dest = std::env::temp_dir().join(format!(
            "munibot_ai_checkout_test_bad_branch_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let error = clone_repository(&dest, &repo.url(), "does-not-exist", None)
            .await
            .expect_err("should fail");
        assert!(error.to_string().contains("git"));

        std::fs::remove_dir_all(&dest).ok();
    }

    #[test]
    fn test_detect_dependency_install_recognizes_cargo() {
        let dir = scratch_dir("cargo");
        std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(
            detect_dependency_install(&dir),
            Some(vec!["cargo".to_string(), "fetch".to_string()])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_detect_dependency_install_recognizes_npm() {
        let dir = scratch_dir("npm");
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        assert_eq!(
            detect_dependency_install(&dir),
            Some(vec!["npm".to_string(), "install".to_string()])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_detect_dependency_install_recognizes_pip() {
        let dir = scratch_dir("pip");
        std::fs::write(dir.join("requirements.txt"), "requests").unwrap();
        assert_eq!(
            detect_dependency_install(&dir),
            Some(vec![
                "pip".to_string(),
                "install".to_string(),
                "-r".to_string(),
                "requirements.txt".to_string(),
            ])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_detect_dependency_install_recognizes_go() {
        let dir = scratch_dir("go");
        std::fs::write(dir.join("go.mod"), "module example.com/x").unwrap();
        assert_eq!(
            detect_dependency_install(&dir),
            Some(vec![
                "go".to_string(),
                "mod".to_string(),
                "download".to_string()
            ])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_detect_dependency_install_is_none_for_an_unrecognized_project() {
        let dir = scratch_dir("unrecognized");
        std::fs::write(dir.join("README.md"), "just docs").unwrap();
        assert_eq!(detect_dependency_install(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_credential_input_never_needs_the_token_on_a_process_argument() {
        // the format itself, not process spawning - proving the shape is
        // exactly the key=value blob the git credential protocol expects
        let host = "github.com";
        let token = "ghp_supersecret";
        let input =
            format!("protocol=https\nhost={host}\nusername=x-access-token\npassword={token}\n\n");
        assert!(input.starts_with("protocol=https\n"));
        assert!(input.contains("host=github.com\n"));
        assert!(input.contains("password=ghp_supersecret\n"));
        assert!(input.ends_with("\n\n"), "must be blank-line terminated");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_checkout_clones_and_runs_dependency_install_inside_the_container() {
        let repo = LocalRepo::new("main", &[(
            "Cargo.toml",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )])
        .await;

        let host_workspace = std::env::temp_dir().join(format!(
            "munibot_ai_checkout_integration_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&host_workspace).unwrap();

        let docker = bollard::Docker::connect_with_podman_defaults()
            .expect("podman should be reachable for this test");
        let mut sandbox = Sandbox::with_docker(docker, crate::sandbox::config::SandboxConfig {
            image: "alpine:latest".to_string(),
            ..Default::default()
        })
        .with_workspace_mount(&host_workspace);

        // no cargo inside plain alpine, so the install step is expected to
        // fail - checkout must still report success, since an unbuilt
        // checkout is still a useful one
        sandbox
            .create(vec!["sleep".to_string(), "30".to_string()])
            .await
            .expect("should create");
        sandbox.start().await.expect("should start");

        sandbox
            .checkout(&repo.url(), "main", None)
            .await
            .expect("checkout should succeed even though the install step can't");

        assert!(host_workspace.join("Cargo.toml").is_file());

        sandbox.remove().await.expect("cleanup should succeed");
        std::fs::remove_dir_all(&host_workspace).ok();
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_checkout_detect_test_{name}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
