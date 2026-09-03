//! The TOML-deserializable shape of `[ai.projects]`.

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

/// The `[ai.projects]` section of munibot's configuration file: where
/// projects live on disk, whether a GitHub App installation registers one
/// automatically, and how long an unused workspace survives.
///
/// Every field has a sensible default - a config file that never mentions
/// `[ai.projects]` at all still gets a working, if generic, setup, the
/// same convention `crate::sandbox::config::SandboxSettings` already
/// established.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectsSettings {
    /// Where every project's bare clone and worktrees live, one directory
    /// per munibot user id below this root. Created on first use (see
    /// `crate::project`'s own project manager, a later commit) rather
    /// than required to pre-exist - an operator pointing this at a fresh
    /// path shouldn't have to `mkdir` it themselves first.
    #[serde(default = "default_projects_root")]
    pub root: PathBuf,
    /// Whether a repository the GitHub App is installed into becomes a
    /// project automatically, the first time munibot sees an event for
    /// it. When `false`, an unregistered repository's webhook deliveries
    /// are ignored with a logged reason rather than silently dropped.
    #[serde(default = "default_auto_register_installations")]
    pub auto_register_installations: bool,
    /// How long a workspace may sit unused before the collection sweep
    /// (a later commit) removes it - checked against a workspace's own
    /// `last_used_at`, and never against one a running pipeline still
    /// holds.
    #[serde(default = "default_workspace_ttl", with = "humantime_serde")]
    pub workspace_ttl: Duration,
    /// How many project images may build concurrently. Nix evaluation is
    /// slow; this is what keeps two cold-cache builds from both starting
    /// at once and thrashing the machine they share.
    #[serde(default = "default_max_concurrent_image_builds")]
    pub max_concurrent_image_builds: usize,
}

fn default_projects_root() -> PathBuf {
    PathBuf::from("/var/lib/munibot/projects")
}

fn default_auto_register_installations() -> bool {
    true
}

fn default_workspace_ttl() -> Duration {
    Duration::from_secs(7 * 24 * 60 * 60)
}

fn default_max_concurrent_image_builds() -> usize {
    1
}

impl Default for ProjectsSettings {
    /// Not `#[derive(Default)]`: none of these fields' sensible defaults
    /// are their own zero value, and a derived `Default` would silently
    /// disagree with what `#[serde(default = "...")]` fills in for a
    /// config file that mentions `[ai.projects]` but leaves a field
    /// unset - the same trap `SandboxSettings::default` and
    /// `AiConfig::default` both document.
    fn default() -> Self {
        Self {
            root: default_projects_root(),
            auto_register_installations: default_auto_register_installations(),
            workspace_ttl: default_workspace_ttl(),
            max_concurrent_image_builds: default_max_concurrent_image_builds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_root_is_the_documented_path() {
        assert_eq!(
            ProjectsSettings::default().root,
            PathBuf::from("/var/lib/munibot/projects")
        );
    }

    #[test]
    fn test_default_auto_registers_installations() {
        assert!(ProjectsSettings::default().auto_register_installations);
    }

    #[test]
    fn test_default_workspace_ttl_is_seven_days() {
        assert_eq!(
            ProjectsSettings::default().workspace_ttl,
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }

    #[test]
    fn test_default_max_concurrent_image_builds_is_one() {
        assert_eq!(ProjectsSettings::default().max_concurrent_image_builds, 1);
    }

    #[test]
    fn test_defaults_every_field_from_an_empty_toml_table() {
        let settings: ProjectsSettings = toml::from_str("").unwrap();
        assert_eq!(settings, ProjectsSettings::default());
    }

    #[test]
    fn test_toml_round_trip_with_every_field_set() {
        let toml_str = r#"
            root = "/srv/munibot/projects"
            auto_register_installations = false
            workspace_ttl = "3d"
            max_concurrent_image_builds = 4
        "#;
        let settings: ProjectsSettings = toml::from_str(toml_str).unwrap();

        assert_eq!(settings.root, PathBuf::from("/srv/munibot/projects"));
        assert!(!settings.auto_register_installations);
        assert_eq!(
            settings.workspace_ttl,
            Duration::from_secs(3 * 24 * 60 * 60)
        );
        assert_eq!(settings.max_concurrent_image_builds, 4);
    }
}
