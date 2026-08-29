//! Normalized forge events.
//!
//! Every forge's own webhook payload is parsed into a [`ForgeEvent`] at the
//! edge (see `munibot_github`'s own webhook handling) -- the pipeline itself
//! never sees a raw GitHub, or eventual GitLab or Forgejo, payload.

use serde::{Deserialize, Serialize};

use crate::{Comment, IssueRef};

/// A single, forge-agnostic thing that happened on a repository munibot
/// watches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ForgeEvent {
    /// A new issue was opened.
    IssueOpened { issue: IssueRef },
    /// An existing issue had a label applied.
    IssueLabeled { issue: IssueRef, label: String },
    /// Someone commented on an issue (or a pull request, which every forge
    /// this crate targets treats as an issue for commenting purposes).
    IssueCommented { issue: IssueRef, comment: Comment },
    /// A pull request munibot opened received a review.
    PullRequestReviewed { issue: IssueRef, comment: Comment },
}

impl ForgeEvent {
    /// The issue or pull request every variant of this event carries a
    /// reference to.
    pub fn issue(&self) -> &IssueRef {
        match self {
            ForgeEvent::IssueOpened { issue }
            | ForgeEvent::IssueLabeled { issue, .. }
            | ForgeEvent::IssueCommented { issue, .. }
            | ForgeEvent::PullRequestReviewed { issue, .. } => issue,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::{Forge, RepoRef};

    fn issue() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn comment() -> Comment {
        Comment {
            author: "someone".to_string(),
            body: "hello".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_issue_opened_carries_its_issue_reference() {
        let event = ForgeEvent::IssueOpened { issue: issue() };
        assert_eq!(event.issue(), &issue());
    }

    #[test]
    fn test_issue_labeled_carries_its_issue_reference() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue(),
            label: "bug".to_string(),
        };
        assert_eq!(event.issue(), &issue());
    }

    #[test]
    fn test_issue_commented_carries_its_issue_reference() {
        let event = ForgeEvent::IssueCommented {
            issue: issue(),
            comment: comment(),
        };
        assert_eq!(event.issue(), &issue());
    }

    #[test]
    fn test_pull_request_reviewed_carries_its_issue_reference() {
        let event = ForgeEvent::PullRequestReviewed {
            issue: issue(),
            comment: comment(),
        };
        assert_eq!(event.issue(), &issue());
    }

    #[test]
    fn test_forge_event_round_trips_through_json() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue(),
            label: "bug".to_string(),
        };
        let encoded = serde_json::to_string(&event).expect("should serialize");
        let decoded: ForgeEvent = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, event);
    }
}
