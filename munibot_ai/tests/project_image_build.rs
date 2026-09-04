//! Integration tests for `ImageBuilder::build_if_needed` composed with
//! `detect_image_strategy`, against real fixture directories and real
//! podman - all three image strategies, exactly as `ProjectManager::register`
//! actually drives them, rather than each build function tested only in
//! isolation. Gated behind `sandbox-integration` like every other
//! podman-touching test in this crate.

use std::path::{Path, PathBuf};

use munibot_ai::project::{ImageBuilder, ImageStatus, ImageStrategy, detect_image_strategy};
use tokio::process::Command;

fn fixture_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "munibot_ai_project_image_build_{name}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join(".munibot")).unwrap();
    dir
}

fn unique_slug(name: &str) -> String {
    format!(
        "test-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

async fn remove_image(tag: &str) {
    let _ = Command::new("podman")
        .args(["rmi", "-f", tag])
        .output()
        .await;
}

fn write_devenv_fixture_with_container_inputs(dir: &Path) {
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

/// The `Containerfile` strategy, end to end: `detect_image_strategy`
/// resolves it with no skip reasons at all (it's tried first), and
/// `build_if_needed` actually builds and tags it against real podman.
#[tokio::test]
#[cfg_attr(not(feature = "sandbox-integration"), ignore)]
async fn test_containerfile_strategy_builds_through_the_real_orchestrator() {
    let dir = fixture_dir("containerfile");
    std::fs::write(dir.join(".munibot/Containerfile"), "FROM alpine:latest\n").unwrap();

    let (strategy, skip_reasons) = detect_image_strategy(&dir);
    assert_eq!(strategy, ImageStrategy::Containerfile);
    assert!(
        skip_reasons.is_empty(),
        "the first strategy tried needs no skip reasons: {skip_reasons:?}"
    );

    let builder = ImageBuilder::new(1);
    let slug = unique_slug("containerfile");
    let status = builder.build_if_needed(&dir, &slug, strategy).await;

    let ImageStatus::Built { tag, .. } = &status else {
        panic!("should build successfully, got {status:?}");
    };
    assert!(tag.starts_with(&format!("munibot-project-{slug}:")));

    remove_image(tag).await;
    std::fs::remove_dir_all(&dir).ok();
}

/// The `Devenv` strategy, end to end, through the same orchestrator - not
/// just `build_devenv_image` directly (already covered in
/// `devenv_image.rs`'s own tests).
#[tokio::test]
#[cfg_attr(not(feature = "sandbox-integration"), ignore)]
async fn test_devenv_strategy_builds_through_the_real_orchestrator() {
    let dir = fixture_dir("devenv");
    write_devenv_fixture_with_container_inputs(&dir);

    let (strategy, skip_reasons) = detect_image_strategy(&dir);
    assert_eq!(strategy, ImageStrategy::Devenv);
    assert_eq!(
        skip_reasons.len(),
        1,
        "Containerfile should be skipped (no .munibot/Containerfile here), Devenv should apply"
    );

    let builder = ImageBuilder::new(1);
    let slug = unique_slug("devenv");
    let status = builder.build_if_needed(&dir, &slug, strategy).await;

    let ImageStatus::Built { tag, .. } = &status else {
        panic!("should build successfully, got {status:?}");
    };
    assert!(tag.starts_with(&format!("munibot-project-{slug}:")));

    remove_image(tag).await;
    std::fs::remove_dir_all(&dir).ok();
}

/// The whole point of a pure strategy-detection function: a `devenv.nix`
/// with no `nix2container`/`mk-shell-bin` inputs declared falls all the
/// way through to `Base`, with a real, actionable skip reason - and
/// `build_if_needed` never touches podman at all for `Base`, since it
/// never builds anything (`[ai.sandbox].image` is used directly).
#[tokio::test]
async fn test_devenv_without_inputs_falls_through_to_base_and_builds_nothing() {
    let dir = fixture_dir("devenv_no_inputs");
    std::fs::write(
        dir.join("devenv.nix"),
        r#"{ pkgs, ... }: { packages = [ pkgs.hello ]; }"#,
    )
    .unwrap();
    // devenv.yaml with neither required input declared
    std::fs::write(
        dir.join("devenv.yaml"),
        r#"inputs:
  nixpkgs:
    url: github:cachix/devenv-nixpkgs/rolling
"#,
    )
    .unwrap();

    let (strategy, skip_reasons) = detect_image_strategy(&dir);
    assert_eq!(strategy, ImageStrategy::Base);
    assert_eq!(
        skip_reasons.len(),
        2,
        "both Containerfile and Devenv should have been tried and skipped: {skip_reasons:?}"
    );
    let devenv_skip = skip_reasons
        .iter()
        .find(|reason| reason.strategy == ImageStrategy::Devenv)
        .expect("Devenv should be among the skipped strategies");
    assert!(
        devenv_skip.reason.contains("nix2container")
            && devenv_skip.reason.contains("mk-shell-bin")
            && devenv_skip.reason.contains("devenv inputs add"),
        "the skip reason should name exactly which inputs are missing and the command to add \
         them: {:?}",
        devenv_skip.reason
    );

    // Base never builds anything at all, so this never touches podman -
    // no #[cfg_attr(not(feature = "sandbox-integration"), ignore)] needed
    let builder = ImageBuilder::new(1);
    let slug = unique_slug("devenv-no-inputs");
    let status = builder.build_if_needed(&dir, &slug, strategy).await;
    assert_eq!(status, ImageStatus::Unbuilt);

    std::fs::remove_dir_all(&dir).ok();
}

/// A repository with neither a `Containerfile` nor a `devenv.nix` at all
/// resolves to `Base` too, with two skip reasons - the common case for a
/// project that opted into neither strategy.
#[tokio::test]
async fn test_a_repository_with_neither_strategy_resolves_to_base() {
    let dir = fixture_dir("plain");
    std::fs::write(dir.join("README.md"), "hello").unwrap();

    let (strategy, skip_reasons) = detect_image_strategy(&dir);
    assert_eq!(strategy, ImageStrategy::Base);
    assert_eq!(skip_reasons.len(), 2);

    let builder = ImageBuilder::new(1);
    let status = builder
        .build_if_needed(&dir, &unique_slug("plain"), strategy)
        .await;
    assert_eq!(status, ImageStatus::Unbuilt);

    std::fs::remove_dir_all(&dir).ok();
}
