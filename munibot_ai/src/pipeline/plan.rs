//! `Plan` and `Subtask`: the pipeline's own tracked representation of a
//! plan, distinct from [`crate::pipeline::CreatePlan`] (what the software
//! architect's handoff produces) because a subtask needs to track its own
//! progress through the build-and-review cycle in a way the architect's
//! own output never has to describe.

use serde::{Deserialize, Serialize};

use crate::pipeline::{CreatePlan, SubtaskDraft, SubtaskId};

/// Where one subtask is in the test-write, test-review, build, code-review,
/// commit cycle.
///
/// Mirrors the schema the architect prompt already emits -- see
/// `munibot_ai/prompts/project-manager.md`'s own description of a
/// subtask's lifecycle, which this type gives a name to at the type level
/// rather than leaving as a string only a prompt agrees on.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SubtaskStatus {
    /// Not yet started.
    Pending,
    /// The test engineer has submitted tests; awaiting test review.
    TestsWritten,
    /// The test reviewer approved the tests as a specification.
    TestsApproved,
    /// The builder is implementing against the approved tests.
    InProgress,
    /// The builder has submitted code; awaiting code review.
    ReviewPending,
    /// The code reviewer approved the implementation.
    Approved,
    /// The commit crafter has committed the approved changes.
    Committed,
}

impl SubtaskStatus {
    /// Whether this subtask has nothing left to do on its own -- committed
    /// is the only status the project manager treats as "done" when
    /// deciding whether every subtask is ready for the final review.
    pub fn is_committed(&self) -> bool {
        matches!(self, SubtaskStatus::Committed)
    }
}

impl Default for SubtaskStatus {
    /// A subtask starts its life having done nothing yet.
    fn default() -> Self {
        SubtaskStatus::Pending
    }
}

/// One subtask the pipeline tracks through its own lifecycle.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Subtask {
    pub id: SubtaskId,
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub commit_message: String,
    pub files_affected: Vec<String>,
    pub dependencies: Vec<SubtaskId>,
    pub status: SubtaskStatus,
}

impl Subtask {
    /// Whether every one of `dependencies` is committed in `all_subtasks`
    /// -- the project manager's own readiness check before starting a
    /// subtask's tests, so a subtask never starts ahead of work it
    /// actually needs.
    pub fn dependencies_satisfied(&self, all_subtasks: &[Subtask]) -> bool {
        self.dependencies.iter().all(|dependency_id| {
            all_subtasks
                .iter()
                .find(|subtask| &subtask.id == dependency_id)
                .is_some_and(|subtask| subtask.status.is_committed())
        })
    }
}

impl From<SubtaskDraft> for Subtask {
    /// A freshly drafted subtask always starts `Pending` -- the architect's
    /// own output never describes progress, since none has happened yet.
    fn from(draft: SubtaskDraft) -> Self {
        Self {
            id: draft.id,
            title: draft.title,
            description: draft.description,
            instructions: draft.instructions,
            commit_message: draft.commit_message,
            files_affected: draft.files_affected,
            dependencies: draft.dependencies,
            status: SubtaskStatus::Pending,
        }
    }
}

/// A plan the pipeline is actively tracking, from architecture review
/// through every subtask's own commit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    pub summary: String,
    pub subtasks: Vec<Subtask>,
}

impl Plan {
    /// Looks a subtask up by id.
    pub fn subtask(&self, id: &SubtaskId) -> Option<&Subtask> {
        self.subtasks.iter().find(|subtask| &subtask.id == id)
    }

    /// Whether every subtask in the plan is committed -- the project
    /// manager's own signal to move on to the final review rather than
    /// starting another subtask.
    pub fn all_subtasks_committed(&self) -> bool {
        self.subtasks
            .iter()
            .all(|subtask| subtask.status.is_committed())
    }
}

impl From<CreatePlan> for Plan {
    fn from(create_plan: CreatePlan) -> Self {
        Self {
            summary: create_plan.summary,
            subtasks: create_plan
                .subtasks
                .into_iter()
                .map(Subtask::from)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtask(id: &str, status: SubtaskStatus, dependencies: Vec<&str>) -> Subtask {
        Subtask {
            id: SubtaskId(id.to_string()),
            title: format!("subtask {id}"),
            description: "does something".to_string(),
            instructions: "do the thing".to_string(),
            commit_message: format!("feat: do the {id} thing"),
            files_affected: vec![],
            dependencies: dependencies
                .into_iter()
                .map(|dependency| SubtaskId(dependency.to_string()))
                .collect(),
            status,
        }
    }

    #[test]
    fn test_subtask_status_defaults_to_pending() {
        assert_eq!(SubtaskStatus::default(), SubtaskStatus::Pending);
    }

    #[test]
    fn test_only_committed_status_is_considered_committed() {
        assert!(SubtaskStatus::Committed.is_committed());
        for status in [
            SubtaskStatus::Pending,
            SubtaskStatus::TestsWritten,
            SubtaskStatus::TestsApproved,
            SubtaskStatus::InProgress,
            SubtaskStatus::ReviewPending,
            SubtaskStatus::Approved,
        ] {
            assert!(!status.is_committed(), "{status:?} should not be committed");
        }
    }

    #[test]
    fn test_subtask_status_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&SubtaskStatus::ReviewPending).unwrap();
        assert_eq!(encoded, "\"review_pending\"");
    }

    #[test]
    fn test_a_subtask_with_no_dependencies_is_always_satisfied() {
        let subtask = subtask("task-1", SubtaskStatus::Pending, vec![]);
        assert!(subtask.dependencies_satisfied(&[]));
    }

    #[test]
    fn test_a_subtask_is_satisfied_when_every_dependency_is_committed() {
        let dependency = subtask("task-1", SubtaskStatus::Committed, vec![]);
        let dependent = subtask("task-2", SubtaskStatus::Pending, vec!["task-1"]);
        assert!(dependent.dependencies_satisfied(&[dependency, dependent.clone()]));
    }

    #[test]
    fn test_a_subtask_is_not_satisfied_when_a_dependency_is_not_committed() {
        let dependency = subtask("task-1", SubtaskStatus::InProgress, vec![]);
        let dependent = subtask("task-2", SubtaskStatus::Pending, vec!["task-1"]);
        assert!(!dependent.dependencies_satisfied(&[dependency, dependent.clone()]));
    }

    #[test]
    fn test_a_subtask_is_not_satisfied_when_the_dependency_is_missing_entirely() {
        let dependent = subtask("task-2", SubtaskStatus::Pending, vec!["ghost-task"]);
        assert!(!dependent.dependencies_satisfied(std::slice::from_ref(&dependent)));
    }

    #[test]
    fn test_plan_subtask_looks_up_by_id() {
        let plan = Plan {
            summary: "a plan".to_string(),
            subtasks: vec![subtask("task-1", SubtaskStatus::Pending, vec![])],
        };
        assert!(plan.subtask(&SubtaskId("task-1".to_string())).is_some());
        assert!(plan.subtask(&SubtaskId("ghost".to_string())).is_none());
    }

    #[test]
    fn test_all_subtasks_committed_is_false_with_any_incomplete_subtask() {
        let plan = Plan {
            summary: "a plan".to_string(),
            subtasks: vec![
                subtask("task-1", SubtaskStatus::Committed, vec![]),
                subtask("task-2", SubtaskStatus::InProgress, vec![]),
            ],
        };
        assert!(!plan.all_subtasks_committed());
    }

    #[test]
    fn test_all_subtasks_committed_is_true_when_every_subtask_is_committed() {
        let plan = Plan {
            summary: "a plan".to_string(),
            subtasks: vec![
                subtask("task-1", SubtaskStatus::Committed, vec![]),
                subtask("task-2", SubtaskStatus::Committed, vec![]),
            ],
        };
        assert!(plan.all_subtasks_committed());
    }

    #[test]
    fn test_all_subtasks_committed_is_true_for_an_empty_plan() {
        let plan = Plan {
            summary: "a plan".to_string(),
            subtasks: vec![],
        };
        assert!(plan.all_subtasks_committed());
    }

    #[test]
    fn test_subtask_draft_converts_into_a_pending_subtask() {
        let draft = SubtaskDraft {
            id: SubtaskId("task-1".to_string()),
            title: "add a toggle".to_string(),
            description: "add a dark mode toggle".to_string(),
            instructions: "add a Toggle component".to_string(),
            commit_message: "feat: add a dark mode toggle".to_string(),
            files_affected: vec!["src/settings.rs".to_string()],
            dependencies: vec![],
        };

        let subtask: Subtask = draft.into();
        assert_eq!(subtask.id, SubtaskId("task-1".to_string()));
        assert_eq!(subtask.status, SubtaskStatus::Pending);
    }

    #[test]
    fn test_create_plan_converts_into_a_plan_with_every_subtask_pending() {
        let create_plan = CreatePlan {
            summary: "add dark mode".to_string(),
            subtasks: vec![SubtaskDraft {
                id: SubtaskId("task-1".to_string()),
                title: "add a toggle".to_string(),
                description: "add a dark mode toggle".to_string(),
                instructions: "add a Toggle component".to_string(),
                commit_message: "feat: add a dark mode toggle".to_string(),
                files_affected: vec![],
                dependencies: vec![],
            }],
        };

        let plan: Plan = create_plan.into();
        assert_eq!(plan.summary, "add dark mode");
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].status, SubtaskStatus::Pending);
    }
}
