//! Detecting a project's image strategy: a pure function from a worktree
//! path to the strategy it resolves to, and why every strategy that was
//! skipped along the way didn't apply.

use std::{collections::HashMap, path::Path};

use serde::{Deserialize, de::IgnoredAny};

use crate::project::ImageStrategy;

/// The two devenv flake inputs a project's `devenv.yaml` must declare for
/// the [`ImageStrategy::Devenv`] strategy to apply - see
/// `docs/plans/ai/milestone-7-projects.md`'s image strategy section.
const REQUIRED_DEVENV_INPUTS: [&str; 2] = ["nix2container", "mk-shell-bin"];

/// Why one strategy, in resolution order, did not apply to a worktree.
///
/// `reason` is a real, actionable sentence - this is exactly what the
/// projects page shows verbatim (a later commit), so it is written like
/// an error message, per `AGENTS.md`: "munibot ignored my devenv" is a bug
/// report; "your devenv.nix is missing two inputs, and here's the exact
/// command to add them" is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipReason {
    pub strategy: ImageStrategy,
    pub reason: String,
}

/// Resolves the image strategy a project's worktree should use, in the
/// order `Containerfile` → `Devenv` → `Base`, along with why every
/// strategy before the one that matched was skipped.
///
/// A pure function over the filesystem: no git, no podman, no nix -
/// exactly what makes this trivially unit-testable against fixture
/// directories and safe to call speculatively (a chat-bound workspace
/// re-detecting its strategy every turn costs nothing).
pub fn detect_image_strategy(worktree: &Path) -> (ImageStrategy, Vec<SkipReason>) {
    let mut skipped = Vec::new();

    if worktree.join(".munibot").join("Containerfile").is_file() {
        return (ImageStrategy::Containerfile, skipped);
    }
    skipped.push(SkipReason {
        strategy: ImageStrategy::Containerfile,
        reason: "no .munibot/Containerfile in this repository".to_string(),
    });

    if !worktree.join("devenv.nix").is_file() {
        skipped.push(SkipReason {
            strategy: ImageStrategy::Devenv,
            reason: "no devenv.nix in this repository".to_string(),
        });
    } else {
        match missing_devenv_container_inputs(worktree) {
            Ok(missing) if missing.is_empty() => return (ImageStrategy::Devenv, skipped),
            Ok(missing) => skipped.push(SkipReason {
                strategy: ImageStrategy::Devenv,
                reason: devenv_skip_reason(&missing),
            }),
            Err(reason) => skipped.push(SkipReason {
                strategy: ImageStrategy::Devenv,
                reason,
            }),
        }
    }

    (ImageStrategy::Base, skipped)
}

/// A real, actionable sentence naming exactly which `devenv inputs add`
/// commands would make the devenv strategy apply - written for a human
/// reading the projects page, not a log grep.
fn devenv_skip_reason(missing: &[&str]) -> String {
    let commands: Vec<String> = missing
        .iter()
        .map(|input| match *input {
            "nix2container" => "devenv inputs add nix2container github:nlewo/nix2container \
                                --follows nixpkgs"
                .to_string(),
            "mk-shell-bin" => {
                "devenv inputs add mk-shell-bin github:rrbutani/nix-mk-shell-bin".to_string()
            }
            other => format!("devenv inputs add {other} <url>"),
        })
        .collect();

    format!(
        "devenv.nix exists, but devenv.yaml is missing the {} input{} the devenv image strategy \
         needs. Add {} by running: {}",
        missing.join(" and "),
        if missing.len() > 1 { "s" } else { "" },
        if missing.len() > 1 { "them" } else { "it" },
        commands.join("; then "),
    )
}

/// The YAML shape of `devenv.yaml` this module actually reads - just
/// enough to see which input names are declared, ignoring everything
/// else about each one (its url, its own nested `inputs.nixpkgs.follows`).
#[derive(Deserialize, Default)]
struct DevenvYaml {
    #[serde(default)]
    inputs: HashMap<String, IgnoredAny>,
}

/// The JSON shape of `devenv.lock` this module actually reads - the
/// top-level `nodes` map's keys are the same input names `devenv.yaml`
/// declares, once devenv has actually resolved them. Checked as well as
/// `devenv.yaml` itself: a project that ran `devenv update` before
/// munibot ever saw it has a lock file that is at least as authoritative
/// as the yaml.
#[derive(serde::Deserialize, Default)]
struct DevenvLock {
    #[serde(default)]
    nodes: HashMap<String, IgnoredAny>,
}

/// Which of [`REQUIRED_DEVENV_INPUTS`] are declared in neither
/// `devenv.yaml` nor `devenv.lock` (when present) - empty means every
/// required input is declared somewhere.
fn missing_devenv_container_inputs(worktree: &Path) -> Result<Vec<&'static str>, String> {
    let yaml_path = worktree.join("devenv.yaml");
    let declared_in_yaml: HashMap<String, IgnoredAny> = if yaml_path.is_file() {
        let contents = std::fs::read_to_string(&yaml_path)
            .map_err(|error| format!("couldn't read devenv.yaml :< {error}"))?;
        let parsed: DevenvYaml = yaml_serde::from_str(&contents)
            .map_err(|error| format!("couldn't parse devenv.yaml :< {error}"))?;
        parsed.inputs
    } else {
        HashMap::new()
    };

    let lock_path = worktree.join("devenv.lock");
    let declared_in_lock: HashMap<String, IgnoredAny> = if lock_path.is_file() {
        let contents = std::fs::read_to_string(&lock_path)
            .map_err(|error| format!("couldn't read devenv.lock :< {error}"))?;
        let parsed: DevenvLock = serde_json::from_str(&contents)
            .map_err(|error| format!("couldn't parse devenv.lock :< {error}"))?;
        parsed.nodes
    } else {
        HashMap::new()
    };

    Ok(REQUIRED_DEVENV_INPUTS
        .into_iter()
        .filter(|input| {
            !declared_in_yaml.contains_key(*input) && !declared_in_lock.contains_key(*input)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "munibot_ai_strategy_fixture_{name}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_containerfile_strategy_wins_when_present_regardless_of_devenv() {
        let dir = fixture_dir("containerfile_wins");
        std::fs::create_dir_all(dir.join(".munibot")).unwrap();
        std::fs::write(dir.join(".munibot/Containerfile"), "FROM alpine").unwrap();
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Containerfile);
        assert!(skipped.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_strategy_applies_when_both_inputs_are_declared() {
        let dir = fixture_dir("devenv_with_inputs");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        std::fs::write(
            dir.join("devenv.yaml"),
            "inputs:\n  nix2container:\n    url: github:nlewo/nix2container\n  mk-shell-bin:\n    \
             url: github:rrbutani/nix-mk-shell-bin\n",
        )
        .unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Devenv);
        assert_eq!(
            skipped.len(),
            1,
            "should record why Containerfile was skipped"
        );
        assert_eq!(skipped[0].strategy, ImageStrategy::Containerfile);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_nix_but_no_inputs_falls_through_to_base_with_an_actionable_reason() {
        let dir = fixture_dir("devenv_no_inputs");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        std::fs::write(
            dir.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:NixOS/nixpkgs\n",
        )
        .unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Base);
        assert_eq!(skipped.len(), 2);

        let devenv_skip = skipped
            .iter()
            .find(|s| s.strategy == ImageStrategy::Devenv)
            .expect("should record why Devenv was skipped");
        assert!(devenv_skip.reason.contains("nix2container"));
        assert!(devenv_skip.reason.contains("mk-shell-bin"));
        assert!(
            devenv_skip
                .reason
                .contains("devenv inputs add nix2container")
        );
        assert!(
            devenv_skip
                .reason
                .contains("devenv inputs add mk-shell-bin")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_nix_with_no_devenv_yaml_at_all_falls_through_to_base() {
        let dir = fixture_dir("devenv_nix_only");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Base);
        let devenv_skip = skipped
            .iter()
            .find(|s| s.strategy == ImageStrategy::Devenv)
            .unwrap();
        assert!(devenv_skip.reason.contains("nix2container"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_missing_one_of_the_two_inputs_names_only_that_one() {
        let dir = fixture_dir("devenv_missing_one");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        std::fs::write(
            dir.join("devenv.yaml"),
            "inputs:\n  nix2container:\n    url: github:nlewo/nix2container\n",
        )
        .unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Base);
        let devenv_skip = skipped
            .iter()
            .find(|s| s.strategy == ImageStrategy::Devenv)
            .unwrap();
        assert!(
            !devenv_skip
                .reason
                .contains("nix2container and mk-shell-bin")
        );
        assert!(devenv_skip.reason.contains("mk-shell-bin"));
        assert!(
            !devenv_skip
                .reason
                .contains("devenv inputs add nix2container")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_devenv_lock_alone_satisfies_the_requirement_without_a_yaml_entry() {
        let dir = fixture_dir("devenv_lock_only");
        std::fs::write(dir.join("devenv.nix"), "{ }").unwrap();
        std::fs::write(
            dir.join("devenv.yaml"),
            "inputs:\n  nixpkgs:\n    url: github:NixOS/nixpkgs\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("devenv.lock"),
            r#"{"nodes": {"nix2container": {}, "mk-shell-bin": {}}}"#,
        )
        .unwrap();

        let (strategy, _) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Devenv);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_neither_containerfile_nor_devenv_resolves_to_base_with_two_skip_reasons() {
        let dir = fixture_dir("plain_base");
        std::fs::write(dir.join("README.md"), "just docs").unwrap();

        let (strategy, skipped) = detect_image_strategy(&dir);
        assert_eq!(strategy, ImageStrategy::Base);
        assert_eq!(skipped.len(), 2);
        assert_eq!(skipped[0].strategy, ImageStrategy::Containerfile);
        assert_eq!(skipped[1].strategy, ImageStrategy::Devenv);

        std::fs::remove_dir_all(&dir).ok();
    }
}
