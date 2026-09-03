//! Building a project's container image from a `.munibot/Containerfile`.

use std::{path::Path, process::Stdio};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tracing::Instrument;

use crate::project::ProjectError;

/// `.munibot/Containerfile`'s path, relative to a worktree's root - see
/// the image strategy table in `docs/plans/ai/milestone-7-projects.md`.
const CONTAINERFILE_RELATIVE_PATH: &str = ".munibot/Containerfile";

/// Builds `image_tag` from `worktree`'s own `.munibot/Containerfile`,
/// streaming `podman build`'s output into a tracing span line by line so
/// a slow build is visible rather than silent.
///
/// Never panics, and never leaves a half-built tag looking real: a
/// failure here returns a [`ProjectError::ImageBuild`] naming the tag,
/// and it is the caller's job (the project manager, a later commit) to
/// record that failure rather than mark the project's image
/// [`crate::project::ImageStatus::Built`].
pub async fn build_containerfile_image(
    worktree: &Path,
    image_tag: &str,
) -> Result<(), ProjectError> {
    let containerfile = worktree.join(CONTAINERFILE_RELATIVE_PATH);
    if !containerfile.is_file() {
        return Err(ProjectError::ImageBuild(format!(
            "{containerfile:?} doesn't exist, but the containerfile strategy was chosen for \
             {image_tag:?}"
        )));
    }

    let span = tracing::info_span!(
        "build_containerfile_image",
        tag = image_tag,
        containerfile = %containerfile.display(),
    );

    async move {
        let mut command = Command::new("podman");
        command
            .args([
                "build",
                "-f",
                &containerfile.display().to_string(),
                "-t",
                image_tag,
                &worktree.display().to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        run_streaming(command, image_tag).await
    }
    .instrument(span)
    .await
}

/// Spawns `command`, streaming each line of its stdout and stderr into
/// the current tracing span as it is produced, and fails naming
/// `subject` (an image tag, in every caller today) on a nonzero exit.
async fn run_streaming(mut command: Command, subject: &str) -> Result<(), ProjectError> {
    let mut child = command
        .spawn()
        .map_err(|error| ProjectError::ImageBuild(format!("couldn't run podman :< {error}")))?;

    let stdout = child
        .stdout
        .take()
        .expect("stdout was piped in build_containerfile_image");
    let stderr = child
        .stderr
        .take()
        .expect("stderr was piped in build_containerfile_image");

    let stdout_task = tokio::spawn(
        async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(%line, "podman build");
            }
        }
        .in_current_span(),
    );
    let stderr_task = tokio::spawn(
        async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(%line, "podman build");
            }
        }
        .in_current_span(),
    );

    let status = child
        .wait()
        .await
        .map_err(|error| ProjectError::ImageBuild(format!("couldn't wait on podman :< {error}")))?;

    // best-effort: the build's own exit status is what actually matters;
    // a reader task failing to join tells us nothing more than the exit
    // status already will
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        return Err(ProjectError::ImageBuild(format!(
            "podman build for {subject:?} exited with {status:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_image_fixture_{name}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".munibot")).unwrap();
        dir
    }

    async fn remove_image_best_effort(tag: &str) {
        let _ = Command::new("podman")
            .args(["rmi", "-f", tag])
            .output()
            .await;
    }

    #[test]
    fn test_missing_containerfile_fails_without_ever_running_podman() {
        let dir = fixture_dir("missing");
        // no .munibot/Containerfile written

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(build_containerfile_image(
                &dir,
                "munibot-project-missing:test",
            ));
        let error = result.expect_err("should fail when the containerfile doesn't exist");
        assert!(error.to_string().contains("Containerfile"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_successful_build_creates_the_tagged_image() {
        let dir = fixture_dir("success");
        std::fs::write(
            dir.join(".munibot/Containerfile"),
            "FROM alpine:latest\nRUN echo hello > /marker.txt\n",
        )
        .unwrap();

        let tag = format!(
            "munibot-project-test-success:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        build_containerfile_image(&dir, &tag)
            .await
            .expect("build should succeed");

        let output = Command::new("podman")
            .args(["image", "exists", &tag])
            .status()
            .await
            .unwrap();
        assert!(output.success(), "the built image should exist locally");

        remove_image_best_effort(&tag).await;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_a_failed_build_never_creates_a_tag() {
        let dir = fixture_dir("failure");
        std::fs::write(
            dir.join(".munibot/Containerfile"),
            "FROM alpine:latest\nRUN exit 1\n",
        )
        .unwrap();

        let tag = format!(
            "munibot-project-test-failure:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let error = build_containerfile_image(&dir, &tag)
            .await
            .expect_err("a RUN exit 1 should fail the build");
        assert!(error.to_string().contains(&tag));

        let output = Command::new("podman")
            .args(["image", "exists", &tag])
            .status()
            .await
            .unwrap();
        assert!(
            !output.success(),
            "a failed build must never leave a tagged image behind"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
