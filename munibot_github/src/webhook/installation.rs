//! Parsing a GitHub App installation webhook delivery - `"installation"`
//! (created/deleted/...) and `"installation_repositories"`
//! (added/removed) - into the plain data a project registration path
//! needs.
//!
//! Deliberately not a `munibot_vcs::ForgeEvent`: neither event describes
//! something happening *to an issue*, which is all `ForgeEvent` can
//! express, and neither carries enough to register a project on its
//! own - GitHub's own installation payloads only ever include a minimal
//! repository object (`full_name`, `private`, an id), never a clone url
//! or a default branch. Resolving those is
//! `GitHubForge::repository_metadata`'s job, a separate call a caller
//! makes for each [`InstalledRepositoryRef`] this module hands back.

use serde::Deserialize;

use crate::error::GitHubError;

/// One repository named in an installation webhook - just enough to
/// look it up for real afterward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledRepositoryRef {
    pub owner: String,
    pub name: String,
}

/// One `"installation"` or `"installation_repositories"` webhook
/// delivery, reduced to which repositories just became newly available
/// or were removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallationEvent {
    pub installation_id: u64,
    pub added: Vec<InstalledRepositoryRef>,
    pub removed: Vec<InstalledRepositoryRef>,
}

#[derive(Deserialize)]
struct RawInstallation {
    id: u64,
}

#[derive(Deserialize)]
struct RawRepository {
    full_name: String,
}

#[derive(Deserialize)]
struct InstallationPayload {
    action: String,
    installation: RawInstallation,
    #[serde(default)]
    repositories: Vec<RawRepository>,
}

#[derive(Deserialize)]
struct InstallationRepositoriesPayload {
    installation: RawInstallation,
    #[serde(default)]
    repositories_added: Vec<RawRepository>,
    #[serde(default)]
    repositories_removed: Vec<RawRepository>,
}

/// Splits `"owner/name"` into its two parts - github's own `full_name`
/// format for every repository object it ever sends. A repository
/// missing the slash entirely is skipped rather than erroring the whole
/// delivery: one malformed entry in a list should never take an entire
/// installation event down with it.
fn split_full_name(full_name: &str) -> Option<InstalledRepositoryRef> {
    let (owner, name) = full_name.split_once('/')?;
    Some(InstalledRepositoryRef {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

fn repo_refs(repos: &[RawRepository]) -> Vec<InstalledRepositoryRef> {
    repos
        .iter()
        .filter_map(|repo| split_full_name(&repo.full_name))
        .collect()
}

/// Parses an `"installation"` or `"installation_repositories"` webhook
/// delivery. Every other event type - including every one
/// [`crate::normalize_webhook`] already handles - returns `Ok(None)`;
/// the two functions are dispatched separately by the caller, never
/// through each other.
pub fn parse_installation_event(
    event_type: &str,
    raw_body: &[u8],
) -> Result<Option<InstallationEvent>, GitHubError> {
    match event_type {
        "installation" => {
            let payload: InstallationPayload = serde_json::from_slice(raw_body)
                .map_err(|error| GitHubError::Payload(error.to_string()))?;
            let repos = repo_refs(&payload.repositories);

            match payload.action.as_str() {
                "created" => Ok(Some(InstallationEvent {
                    installation_id: payload.installation.id,
                    added: repos,
                    removed: Vec::new(),
                })),
                "deleted" => Ok(Some(InstallationEvent {
                    installation_id: payload.installation.id,
                    added: Vec::new(),
                    removed: repos,
                })),
                // new_permissions_accepted, suspend, unsuspend: none of
                // these change which repositories are registerable
                _ => Ok(None),
            }
        }
        "installation_repositories" => {
            let payload: InstallationRepositoriesPayload = serde_json::from_slice(raw_body)
                .map_err(|error| GitHubError::Payload(error.to_string()))?;
            Ok(Some(InstallationEvent {
                installation_id: payload.installation.id,
                added: repo_refs(&payload.repositories_added),
                removed: repo_refs(&payload.repositories_removed),
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installation_created_reports_every_repository_as_added() {
        let body = br#"{
            "action": "created",
            "installation": { "id": 42 },
            "repositories": [
                { "id": 1, "full_name": "muni-corn/cocoa", "private": false },
                { "id": 2, "full_name": "muni-corn/munibot", "private": true }
            ]
        }"#;

        let event = parse_installation_event("installation", body)
            .expect("should parse")
            .expect("should be Some");

        assert_eq!(event.installation_id, 42);
        assert_eq!(event.added, vec![
            InstalledRepositoryRef {
                owner: "muni-corn".to_string(),
                name: "cocoa".to_string()
            },
            InstalledRepositoryRef {
                owner: "muni-corn".to_string(),
                name: "munibot".to_string()
            },
        ]);
        assert!(event.removed.is_empty());
    }

    #[test]
    fn test_installation_deleted_reports_every_repository_as_removed() {
        let body = br#"{
            "action": "deleted",
            "installation": { "id": 42 },
            "repositories": [
                { "id": 1, "full_name": "muni-corn/cocoa", "private": false }
            ]
        }"#;

        let event = parse_installation_event("installation", body)
            .expect("should parse")
            .expect("should be Some");

        assert!(event.added.is_empty());
        assert_eq!(event.removed.len(), 1);
    }

    #[test]
    fn test_installation_suspend_is_not_interesting() {
        let body = br#"{
            "action": "suspend",
            "installation": { "id": 42 },
            "repositories": []
        }"#;

        assert!(
            parse_installation_event("installation", body)
                .expect("should parse")
                .is_none()
        );
    }

    #[test]
    fn test_installation_repositories_added_and_removed_are_both_reported() {
        let body = br#"{
            "action": "added",
            "installation": { "id": 7 },
            "repositories_added": [
                { "id": 1, "full_name": "muni-corn/cocoa", "private": false }
            ],
            "repositories_removed": [
                { "id": 2, "full_name": "muni-corn/old-project", "private": false }
            ]
        }"#;

        let event = parse_installation_event("installation_repositories", body)
            .expect("should parse")
            .expect("should be Some");

        assert_eq!(event.installation_id, 7);
        assert_eq!(event.added[0].name, "cocoa");
        assert_eq!(event.removed[0].name, "old-project");
    }

    #[test]
    fn test_an_unrelated_event_type_is_not_interesting() {
        assert!(
            parse_installation_event("issues", b"{}")
                .expect("should parse")
                .is_none()
        );
    }

    #[test]
    fn test_a_malformed_body_is_a_payload_error_not_a_panic() {
        let error = parse_installation_event("installation", b"not json").unwrap_err();
        assert!(matches!(error, GitHubError::Payload(_)));
    }

    #[test]
    fn test_a_repository_full_name_with_no_slash_is_skipped_not_fatal() {
        let body = br#"{
            "action": "created",
            "installation": { "id": 42 },
            "repositories": [
                { "id": 1, "full_name": "malformed-no-slash", "private": false },
                { "id": 2, "full_name": "muni-corn/cocoa", "private": false }
            ]
        }"#;

        let event = parse_installation_event("installation", body)
            .expect("should parse")
            .expect("should be Some");
        assert_eq!(event.added.len(), 1);
        assert_eq!(event.added[0].name, "cocoa");
    }
}
