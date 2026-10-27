//! Building a project's container image from its own `devenv.nix`, via
//! `nix2container`.

use std::{path::Path, process::Stdio};

use tokio::process::Command;
use tracing::Instrument;

use crate::project::ProjectError;

/// The package name every devenv container build produces, per
/// `docs/plans/ai/milestone-7-projects.md`'s image strategy table -
/// `devenv container build shell` and `devenv container copy ... shell`
/// both name it explicitly rather than devenv choosing one.
const DEVENV_CONTAINER_NAME: &str = "shell";

/// Builds `image_tag` from `worktree`'s own `devenv.nix`: `devenv
/// container build shell`, then `devenv container copy --registry
/// containers-storage: shell` (lands it where local podman actually reads
/// from - see `docs/notes/projects-preflight-findings.md` for why the
/// registry flag's position matters and what a rootless host needs for
/// this to work at all), then retags the result to `image_tag`.
///
/// Streams both devenv subprocesses' output into a tracing span line by
/// line - nix evaluation is slow, and this is the one place in the whole
/// milestone where that matters most. Never leaves a half-built tag
/// looking real: a failure at either step returns a
/// [`ProjectError::ImageBuild`] before `image_tag` is ever created.
pub async fn build_devenv_image(worktree: &Path, image_tag: &str) -> Result<(), ProjectError> {
    let span = tracing::info_span!("build_devenv_image", tag = image_tag);

    async move {
        run_streaming(devenv_command(worktree, &[
            "container",
            "build",
            DEVENV_CONTAINER_NAME,
        ]))
        .await?;
        run_streaming(devenv_command(worktree, &[
            "container",
            "copy",
            "--registry",
            "containers-storage:",
            DEVENV_CONTAINER_NAME,
        ]))
        .await?;

        // devenv always names the image `shell:latest`; retag it to what
        // the caller actually asked for, then drop the temporary name so
        // concurrent builds of *other* projects (serialized by
        // max_concurrent_image_builds, a later commit) never collide on
        // it holding a stale reference
        run_streaming(podman_command(&["tag", "shell:latest", image_tag])).await?;
        let _ = run_streaming(podman_command(&["rmi", "shell:latest"])).await;

        Ok(())
    }
    .instrument(span)
    .await
}

fn devenv_command(worktree: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("devenv");
    command
        .current_dir(worktree)
        .args(args)
        // devenv builds shell out to nix, which - through secretspec's
        // own agent-access policy - refuses to run without a stated
        // reason; a project image build is exactly that reason
        .env(
            "SECRETSPEC_REASON",
            "building a project's devenv container image",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn podman_command(args: &[&str]) -> Command {
    let mut command = Command::new("podman");
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Spawns `command`, streaming each line of its stdout and stderr into
/// the current tracing span as it is produced, and fails naming the
/// command's own first argument on a nonzero exit.
async fn run_streaming(mut command: Command) -> Result<(), ProjectError> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = command.spawn().map_err(|error| {
        ProjectError::ImageBuild(format!("couldn't run the build step :< {error}"))
    })?;

    let stdout = child.stdout.take().expect("stdout was piped above");
    let stderr = child.stderr.take().expect("stderr was piped above");

    let stdout_task = tokio::spawn(
        async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(%line, "devenv container build");
            }
        }
        .in_current_span(),
    );
    let stderr_task = tokio::spawn(
        async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(%line, "devenv container build");
            }
        }
        .in_current_span(),
    );

    let status = child.wait().await.map_err(|error| {
        ProjectError::ImageBuild(format!("couldn't wait on the build step :< {error}"))
    })?;

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        return Err(ProjectError::ImageBuild(format!(
            "a devenv image build step exited with {status:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal devenv project declaring the two container inputs -
    /// never munibot's own `devenv.yaml`, per the milestone's own rule
    /// that a project's repository is never mutated to make a strategy
    /// work.
    fn write_devenv_fixture(dir: &Path) {
        std::fs::write(
            dir.join("devenv.yaml"),
            r#"inputs:
  nixpkgs:
    url: github:cachix/devenv-nixpkgs/rolling
  nix2container:
    url: github:nlewo/nix2container
    inputs:
      nixpkgs:
        follows: nixpkgs
  mk-shell-bin:
    url: github:rrbutani/nix-mk-shell-bin
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("devenv.nix"),
            r#"{ pkgs, ... }: { packages = [ pkgs.hello ]; }"#,
        )
        .unwrap();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_build_devenv_image_produces_a_runnable_tagged_image() {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_devenv_image_fixture_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        write_devenv_fixture(&dir);

        let tag = format!(
            "munibot-project-test-devenv:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        build_devenv_image(&dir, &tag)
            .await
            .expect("devenv image build should succeed");

        let output = Command::new("podman")
            .args(["run", "--rm", &tag, "hello"])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Hello, world!"));

        let _ = Command::new("podman")
            .args(["rmi", "-f", &tag])
            .output()
            .await;
        std::fs::remove_dir_all(&dir).ok();
    }
}
