//! Per-repository trigger configuration: how a repository owner chooses
//! which forge events actually wake the pipeline up.

use serde::{Deserialize, Serialize};

use crate::{ForgeEvent, RepoRef};

/// How a repository decides which events should start a pipeline run.
///
/// A pure description of a matching policy, not the policy's own
/// enforcement -- see [`TriggerMode::matches`] for that. Repository owners
/// choose their own trigger style: some want every issue triaged, some only
/// want issues a maintainer has explicitly labelled, some want a keyword in
/// the body, and some want any of several conditions to qualify.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TriggerMode {
    /// Every issue opened on the repository.
    AllIssues,
    /// Only issues carrying this exact label.
    Label(String),
    /// Only issues whose title or body contains this keyword
    /// (case-insensitive).
    Keyword(String),
    /// Any one of several modes qualifies.
    Any(Vec<TriggerMode>),
}

impl TriggerMode {
    /// Whether `event` satisfies this trigger, given the issue's own title
    /// and body for [`TriggerMode::Keyword`] to search.
    ///
    /// A pure function: no I/O, no forge lookups of its own. Keyword and
    /// label matching both need text the event itself doesn't necessarily
    /// carry (an `IssueLabeled` event names the label that was just
    /// applied, but a keyword trigger needs the issue's title and body
    /// regardless of which event fired), so the caller supplies both
    /// alongside the event rather than this function fetching them itself.
    pub fn matches(&self, event: &ForgeEvent, title: &str, body: &str) -> bool {
        match self {
            TriggerMode::AllIssues => matches!(event, ForgeEvent::IssueOpened { .. }),
            TriggerMode::Label(label) => matches!(
                event,
                ForgeEvent::IssueLabeled { label: applied, .. } if applied == label
            ),
            TriggerMode::Keyword(keyword) => {
                let keyword = keyword.to_lowercase();
                title.to_lowercase().contains(&keyword) || body.to_lowercase().contains(&keyword)
            }
            TriggerMode::Any(modes) => modes.iter().any(|mode| mode.matches(event, title, body)),
        }
    }
}

/// One repository's own trigger configuration.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RepoTriggerConfig {
    pub repo: RepoRef,
    pub mode: TriggerMode,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Comment, Forge, IssueRef};

    fn issue_ref() -> IssueRef {
        IssueRef::new(RepoRef::new(Forge::GitHub, "musicaloft", "munibot"), 1)
    }

    fn comment() -> Comment {
        Comment {
            author: "someone".to_string(),
            body: "hello".to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_all_issues_matches_issue_opened() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(TriggerMode::AllIssues.matches(&event, "title", "body"));
    }

    #[test]
    fn test_all_issues_does_not_match_a_comment() {
        let event = ForgeEvent::IssueCommented {
            issue: issue_ref(),
            comment: comment(),
        };
        assert!(!TriggerMode::AllIssues.matches(&event, "title", "body"));
    }

    #[test]
    fn test_label_matches_the_exact_label_applied() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue_ref(),
            label: "ai-triage".to_string(),
        };
        assert!(TriggerMode::Label("ai-triage".to_string()).matches(&event, "title", "body"));
    }

    #[test]
    fn test_label_does_not_match_a_different_label() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue_ref(),
            label: "wontfix".to_string(),
        };
        assert!(!TriggerMode::Label("ai-triage".to_string()).matches(&event, "title", "body"));
    }

    #[test]
    fn test_label_does_not_match_issue_opened() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(!TriggerMode::Label("ai-triage".to_string()).matches(&event, "title", "body"));
    }

    #[test]
    fn test_keyword_matches_in_the_title_case_insensitively() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(TriggerMode::Keyword("CRASH".to_string()).matches(
            &event,
            "app crashes on start",
            ""
        ));
    }

    #[test]
    fn test_keyword_matches_in_the_body() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(TriggerMode::Keyword("regression".to_string()).matches(
            &event,
            "title",
            "this looks like a regression from last release"
        ));
    }

    #[test]
    fn test_keyword_does_not_match_when_absent_from_both() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(!TriggerMode::Keyword("regression".to_string()).matches(&event, "title", "body"));
    }

    #[test]
    fn test_any_matches_when_one_branch_matches() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue_ref(),
            label: "ai-triage".to_string(),
        };
        let mode = TriggerMode::Any(vec![
            TriggerMode::Keyword("regression".to_string()),
            TriggerMode::Label("ai-triage".to_string()),
        ]);
        assert!(mode.matches(&event, "title", "body"));
    }

    #[test]
    fn test_any_does_not_match_when_no_branch_matches() {
        let event = ForgeEvent::IssueLabeled {
            issue: issue_ref(),
            label: "wontfix".to_string(),
        };
        let mode = TriggerMode::Any(vec![
            TriggerMode::Keyword("regression".to_string()),
            TriggerMode::Label("ai-triage".to_string()),
        ]);
        assert!(!mode.matches(&event, "title", "body"));
    }

    #[test]
    fn test_any_with_no_branches_never_matches() {
        let event = ForgeEvent::IssueOpened { issue: issue_ref() };
        assert!(!TriggerMode::Any(vec![]).matches(&event, "title", "body"));
    }

    #[test]
    fn test_repo_trigger_config_round_trips_through_json() {
        let config = RepoTriggerConfig {
            repo: RepoRef::new(Forge::GitHub, "musicaloft", "munibot"),
            mode: TriggerMode::Label("ai-triage".to_string()),
            enabled: true,
        };
        let encoded = serde_json::to_string(&config).expect("should serialize");
        let decoded: RepoTriggerConfig =
            serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, config);
    }
}
