//! Repository, issue, and comment reference types every forge integration
//! normalizes into.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which forge a [`RepoRef`] belongs to.
///
/// Deliberately an enum rather than a free-text string: the pipeline
/// dispatches on this exactly once (choosing which `Arc<dyn IssueSource>` to
/// route to), and a typo in a forge name should be a compile error, not a
/// silently-ignored webhook.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Forge {
    #[serde(rename = "github")]
    GitHub,
}

impl fmt::Display for Forge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Forge::GitHub => write!(f, "github"),
        }
    }
}

/// A repository on a specific forge, such as `github:musicaloft/munibot`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub forge: Forge,
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn new(forge: Forge, owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            forge,
            owner: owner.into(),
            name: name.into(),
        }
    }
}

/// The conventional `owner/name` form, with no forge prefix -- this is the
/// shape a person recognizes from the forge's own UI, a commit trailer, or a
/// pull request title, so it is what every user-facing surface (the
/// pipeline monitor, a chat message asking a clarifying question) should
/// show rather than [`RepoRef::forge`]-qualified debug output.
impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// One issue on one repository, identified by its number.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct IssueRef {
    pub repo: RepoRef,
    pub number: u64,
}

impl IssueRef {
    pub fn new(repo: RepoRef, number: u64) -> Self {
        Self { repo, number }
    }
}

/// The conventional `owner/name#number` form.
impl fmt::Display for IssueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

/// Whether an issue is open or closed, at the moment it was fetched.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

/// A normalized issue, regardless of which forge it came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub reference: IssueRef,
    pub title: String,
    /// The issue's own description. Attacker-controlled: this text comes
    /// from anyone who can open an issue on a public repository, and it
    /// reaches an agent holding filesystem and shell tools once the
    /// pipeline starts. Callers must run it through the untrusted-content
    /// wrapper (see `munibot_ai::tools::untrusted`) before it ever reaches a
    /// model, never pass it through directly.
    pub body: String,
    pub author: String,
    pub labels: Vec<String>,
    pub state: IssueState,
}

/// A single comment on an issue or pull request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Comment {
    pub author: String,
    /// Attacker-controlled for the same reason [`Issue::body`] is -- see
    /// that field's own doc comment.
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> RepoRef {
        RepoRef::new(Forge::GitHub, "musicaloft", "munibot")
    }

    #[test]
    fn test_forge_displays_lowercase() {
        assert_eq!(Forge::GitHub.to_string(), "github");
    }

    #[test]
    fn test_repo_ref_displays_as_owner_slash_name() {
        assert_eq!(repo().to_string(), "musicaloft/munibot");
    }

    #[test]
    fn test_issue_ref_displays_as_owner_slash_name_hash_number() {
        let issue_ref = IssueRef::new(repo(), 42);
        assert_eq!(issue_ref.to_string(), "musicaloft/munibot#42");
    }

    #[test]
    fn test_repo_ref_serializes_and_round_trips() {
        let encoded = serde_json::to_string(&repo()).expect("should serialize");
        let decoded: RepoRef = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, repo());
    }

    #[test]
    fn test_forge_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&Forge::GitHub).expect("should serialize");
        assert_eq!(encoded, "\"github\"");
    }

    #[test]
    fn test_issue_state_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&IssueState::Open).expect("should serialize");
        assert_eq!(encoded, "\"open\"");
    }

    #[test]
    fn test_issue_round_trips_through_json() {
        let issue = Issue {
            reference: IssueRef::new(repo(), 7),
            title: "something is broken".to_string(),
            body: "steps to reproduce...".to_string(),
            author: "someone".to_string(),
            labels: vec!["bug".to_string()],
            state: IssueState::Open,
        };

        let encoded = serde_json::to_string(&issue).expect("should serialize");
        let decoded: Issue = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, issue);
    }

    #[test]
    fn test_comment_round_trips_through_json() {
        let comment = Comment {
            author: "someone".to_string(),
            body: "can you clarify?".to_string(),
            created_at: Utc::now(),
        };

        let encoded = serde_json::to_string(&comment).expect("should serialize");
        let decoded: Comment = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, comment);
    }
}
