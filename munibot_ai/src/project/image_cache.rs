//! Computing a project image's source hash, checking whether that exact
//! tag already exists locally, and serializing concurrent builds behind
//! a configured limit.

use std::{path::Path, sync::Arc};

use sha2::{Digest, Sha256};
use tokio::{process::Command, sync::Semaphore};

use crate::project::{
    ImageStatus, ImageStrategy, ProjectError, build_containerfile_image, build_devenv_image,
};

/// Bumped whenever munibot's own image contract changes (the tool agent
/// mount path, what `tool_agent_cmd` passes) - folded into every source
/// hash so a project's cached image is invalidated the moment munibot
/// itself would no longer know how to use it, even though nothing in the
/// project's own repository changed at all.
const IMAGE_CONTRACT_SALT: &str = "munibot-image-contract-v1";

/// Which files feed a devenv image's source hash - `devenv.lock` is
/// included because it pins the exact resolved versions of
/// `devenv.yaml`'s own inputs; two commits with an identical `devenv.nix`
/// and `devenv.yaml` but a `devenv update`'d lock file are not the same
/// image.
const DEVENV_HASH_INPUTS: [&str; 3] = ["devenv.nix", "devenv.yaml", "devenv.lock"];

/// The first sixteen hex characters of a SHA-256 over `strategy`'s own
/// declared inputs, plus [`IMAGE_CONTRACT_SALT`] - see
/// `docs/plans/ai/milestone-7-projects.md`'s image strategy section.
///
/// A pure function over the filesystem for `Containerfile`/`Devenv`;
/// `Base` has no project-specific inputs at all; since a `Base`-strategy
/// project never builds anything (`[ai.sandbox].image` is used directly),
/// this still returns a stable value rather than an error, so a caller
/// that computes a hash speculatively before checking the strategy never
/// has to special-case it.
pub fn compute_source_hash(
    strategy: ImageStrategy,
    worktree: &Path,
) -> Result<String, ProjectError> {
    let mut hasher = Sha256::new();
    hasher.update(IMAGE_CONTRACT_SALT.as_bytes());

    match strategy {
        ImageStrategy::Containerfile => {
            let path = worktree.join(".munibot/Containerfile");
            let bytes = std::fs::read(&path).map_err(|error| {
                ProjectError::Other(format!("couldn't read {path:?} :< {error}"))
            })?;
            hasher.update(&bytes);
        }
        ImageStrategy::Devenv => {
            for name in DEVENV_HASH_INPUTS {
                // devenv.lock may not exist (a project that has never run
                // `devenv update`); every other input is required by the
                // strategy already having been chosen, so a missing one
                // here would mean detection and hashing disagreed
                if let Ok(bytes) = std::fs::read(worktree.join(name)) {
                    hasher.update(name.as_bytes());
                    hasher.update(&bytes);
                }
            }
        }
        ImageStrategy::Base => {
            hasher.update(b"base");
        }
    }

    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(hex[..16].to_string())
}

/// The image tag a project's own strategy resolves to, given its slug
/// and source hash - `munibot-project-<slug>:<hash>`.
pub fn image_tag(slug: &str, hash: &str) -> String {
    format!("munibot-project-{slug}:{hash}")
}

/// When `tag` exists in the local podman image store, per `podman image
/// inspect`'s own `Created` field - `None` if it does not exist at all.
async fn locally_built_at(
    tag: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ProjectError> {
    let output = Command::new("podman")
        .args(["image", "inspect", tag, "--format", "{{.Created}}"])
        .output()
        .await
        .map_err(|error| ProjectError::Other(format!("couldn't run podman :< {error}")))?;

    if !output.status.success() {
        // podman's own exit code for "no such image" is indistinguishable
        // from other inspect failures without parsing stderr text, which
        // is not worth depending on across podman versions - either way,
        // "couldn't confirm it exists" means "build it"
        return Ok(None);
    }

    let created = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // podman's Go-formatted timestamp, e.g. "2026-06-16 00:01:29.967161902
    // +0000 UTC" - not RFC3339, so this parses it explicitly rather than
    // trusting a generic parser to guess the shape
    let without_zone_name = created.trim_end_matches(" UTC");
    chrono::DateTime::parse_from_str(without_zone_name, "%Y-%m-%d %H:%M:%S%.f %z")
        .map(|dt| Some(dt.with_timezone(&chrono::Utc)))
        .map_err(|error| {
            ProjectError::Other(format!(
                "couldn't parse podman's own timestamp {created:?} :< {error}"
            ))
        })
}

/// Builds project images, skipping a build when the exact tag its source
/// hash resolves to already exists locally, and serializing actual
/// builds behind `[ai.projects].max_concurrent_image_builds` - two cold
/// nix evaluations at once on one machine is how you discover what swap
/// is for.
pub struct ImageBuilder {
    semaphore: Arc<Semaphore>,
}

impl ImageBuilder {
    pub fn new(max_concurrent_image_builds: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent_image_builds.max(1))),
        }
    }

    /// How many build slots are free right now - for diagnostics and
    /// tests, not something a caller needs to check before calling
    /// [`Self::build_if_needed`], which waits for one itself.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Resolves `slug`'s image for `worktree` under `strategy`: computes
    /// the source hash, skips the actual build if that exact tag already
    /// exists locally, and otherwise builds it - waiting for a free slot
    /// first if every one is already in use.
    ///
    /// Never propagates a failure: every error becomes
    /// [`ImageStatus::Failed`], since a failed build is a normal, expected
    /// outcome this milestone's own definition of done calls for
    /// recording on the project row, not for panicking or losing context
    /// through a bare `Result`.
    pub async fn build_if_needed(
        &self,
        worktree: &Path,
        slug: &str,
        strategy: ImageStrategy,
    ) -> ImageStatus {
        if strategy == ImageStrategy::Base {
            return ImageStatus::Unbuilt;
        }

        let hash = match compute_source_hash(strategy, worktree) {
            Ok(hash) => hash,
            Err(error) => {
                return ImageStatus::Failed {
                    reason: error.to_string(),
                };
            }
        };
        let tag = image_tag(slug, &hash);

        match locally_built_at(&tag).await {
            Ok(Some(at)) => return ImageStatus::Built { tag, hash, at },
            Ok(None) => {}
            Err(error) => {
                return ImageStatus::Failed {
                    reason: error.to_string(),
                };
            }
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .expect("this semaphore is never closed");

        let build_result = match strategy {
            ImageStrategy::Containerfile => build_containerfile_image(worktree, &tag).await,
            ImageStrategy::Devenv => build_devenv_image(worktree, &tag).await,
            ImageStrategy::Base => unreachable!("returned above"),
        };

        if let Err(error) = build_result {
            return ImageStatus::Failed {
                reason: error.to_string(),
            };
        }

        match locally_built_at(&tag).await {
            Ok(Some(at)) => ImageStatus::Built { tag, hash, at },
            Ok(None) => ImageStatus::Failed {
                reason: format!(
                    "podman build for {tag:?} reported success, but the image doesn't exist \
                     afterward"
                ),
            },
            Err(error) => ImageStatus::Failed {
                reason: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_image_cache_fixture_{name}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".munibot")).unwrap();
        dir
    }

    #[test]
    fn test_base_strategy_hash_is_stable_regardless_of_worktree_contents() {
        let dir = fixture_dir("base_stable");
        let first = compute_source_hash(ImageStrategy::Base, &dir).unwrap();
        std::fs::write(dir.join("anything.txt"), "content").unwrap();
        let second = compute_source_hash(ImageStrategy::Base, &dir).unwrap();
        assert_eq!(first, second);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_containerfile_hash_changes_when_the_containerfile_changes() {
        let dir = fixture_dir("containerfile_hash");
        std::fs::write(dir.join(".munibot/Containerfile"), "FROM alpine:latest\n").unwrap();
        let first = compute_source_hash(ImageStrategy::Containerfile, &dir).unwrap();

        std::fs::write(
            dir.join(".munibot/Containerfile"),
            "FROM debian:bookworm-slim\n",
        )
        .unwrap();
        let second = compute_source_hash(ImageStrategy::Containerfile, &dir).unwrap();

        assert_ne!(first, second);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_containerfile_hash_is_stable_for_unchanged_content() {
        let dir = fixture_dir("containerfile_stable");
        std::fs::write(dir.join(".munibot/Containerfile"), "FROM alpine:latest\n").unwrap();
        let first = compute_source_hash(ImageStrategy::Containerfile, &dir).unwrap();
        let second = compute_source_hash(ImageStrategy::Containerfile, &dir).unwrap();
        assert_eq!(first, second);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_hash_changes_when_devenv_nix_changes() {
        let dir = fixture_dir("devenv_hash");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        let first = compute_source_hash(ImageStrategy::Devenv, &dir).unwrap();

        std::fs::write(dir.join("devenv.nix"), "{ packages = []; }").unwrap();
        let second = compute_source_hash(ImageStrategy::Devenv, &dir).unwrap();

        assert_ne!(first, second);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_hash_changes_when_only_the_lock_file_changes() {
        let dir = fixture_dir("devenv_lock_hash");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        std::fs::write(dir.join("devenv.lock"), r#"{"nodes": {}}"#).unwrap();
        let first = compute_source_hash(ImageStrategy::Devenv, &dir).unwrap();

        std::fs::write(dir.join("devenv.lock"), r#"{"nodes": {"nixpkgs": {}}}"#).unwrap();
        let second = compute_source_hash(ImageStrategy::Devenv, &dir).unwrap();

        assert_ne!(
            first, second,
            "a devenv update changing only the lock file must invalidate the cache"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_image_tag_matches_the_documented_format() {
        assert_eq!(image_tag("cocoa", "abc123"), "munibot-project-cocoa:abc123");
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_an_unchanged_input_builds_once_and_a_changed_one_builds_twice() {
        let dir = fixture_dir("cache_behavior");
        std::fs::write(dir.join(".munibot/Containerfile"), "FROM alpine:latest\n").unwrap();

        let builder = ImageBuilder::new(1);
        let slug = format!(
            "test-cache-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let first = builder
            .build_if_needed(&dir, &slug, ImageStrategy::Containerfile)
            .await;
        let ImageStatus::Built {
            tag: first_tag,
            at: first_at,
            ..
        } = first
        else {
            panic!("first build should succeed, got {first:?}");
        };

        // calling again with the exact same input must be a cache hit:
        // the same tag, and the same build time (a rebuild would produce
        // a new Created timestamp)
        let second = builder
            .build_if_needed(&dir, &slug, ImageStrategy::Containerfile)
            .await;
        let ImageStatus::Built {
            tag: second_tag,
            at: second_at,
            ..
        } = second
        else {
            panic!("second call should still resolve to Built, got {second:?}");
        };
        assert_eq!(first_tag, second_tag);
        assert_eq!(first_at, second_at, "a cache hit must not rebuild");

        // changing the input must produce a different tag, built fresh
        std::fs::write(
            dir.join(".munibot/Containerfile"),
            "FROM debian:bookworm-slim\n",
        )
        .unwrap();
        let third = builder
            .build_if_needed(&dir, &slug, ImageStrategy::Containerfile)
            .await;
        let ImageStatus::Built { tag: third_tag, .. } = third else {
            panic!("third build (changed input) should succeed, got {third:?}");
        };
        assert_ne!(
            first_tag, third_tag,
            "a changed input must resolve to a different tag"
        );

        let _ = Command::new("podman")
            .args(["rmi", "-f", &first_tag, &third_tag])
            .output()
            .await;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_builds_are_serialized_behind_max_concurrent_image_builds() {
        let dir_a = fixture_dir("concurrency_a");
        let dir_b = fixture_dir("concurrency_b");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // a slow build: proves the second build waits rather than running
        // alongside it. The sleep's own argument is unique per test run
        // so podman's build-layer cache (keyed on instruction text, not
        // file path) can never turn this into a cache hit against an
        // earlier run of this exact test.
        std::fs::write(
            dir_a.join(".munibot/Containerfile"),
            format!("FROM alpine:latest\nRUN sleep 2 && echo {nanos}\n"),
        )
        .unwrap();
        std::fs::write(dir_b.join(".munibot/Containerfile"), "FROM alpine:latest\n").unwrap();

        let builder = Arc::new(ImageBuilder::new(1));
        let slug_a = format!("test-concurrency-a-{nanos}");
        let slug_b = format!("test-concurrency-b-{nanos}");

        let builder_a = Arc::clone(&builder);
        let dir_a_clone = dir_a.clone();
        let slug_a_clone = slug_a.clone();
        let handle_a = tokio::spawn(async move {
            builder_a
                .build_if_needed(&dir_a_clone, &slug_a_clone, ImageStrategy::Containerfile)
                .await
        });

        // poll rather than a single fixed sleep: the first build still
        // has to run compute_source_hash and a podman image inspect
        // before it even reaches the semaphore, and how long that takes
        // depends on host load, not just wall-clock guesswork
        let mut waited = std::time::Duration::ZERO;
        while builder.available_permits() > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            waited += std::time::Duration::from_millis(50);
            assert!(
                waited < std::time::Duration::from_secs(5),
                "the slow build never acquired the only permit"
            );
        }

        let started_b = std::time::Instant::now();
        let result_b = builder
            .build_if_needed(&dir_b, &slug_b, ImageStrategy::Containerfile)
            .await;
        let elapsed_b = started_b.elapsed();

        let result_a = handle_a.await.unwrap();

        assert!(matches!(result_a, ImageStatus::Built { .. }));
        assert!(matches!(result_b, ImageStatus::Built { .. }));
        assert!(
            elapsed_b >= std::time::Duration::from_millis(1200),
            "the second build should have waited for the first to release its permit, took \
             {elapsed_b:?}"
        );

        if let ImageStatus::Built { tag, .. } = result_a {
            let _ = Command::new("podman")
                .args(["rmi", "-f", &tag])
                .output()
                .await;
        }
        if let ImageStatus::Built { tag, .. } = result_b {
            let _ = Command::new("podman")
                .args(["rmi", "-f", &tag])
                .output()
                .await;
        }
        std::fs::remove_dir_all(&dir_a).ok();
        std::fs::remove_dir_all(&dir_b).ok();
    }
}
