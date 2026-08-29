//! The machine-readable output contract per pipeline role.
//!
//! The twelve prompts themselves are not written here -- they were ported
//! into `munibot_ai/prompts` deliberately with their own output-contract
//! prose stripped out (see `docs/notes/persona-prompt-porting.md`), so
//! that a [`crate::harness::HandoffSchema`] could supply it exactly once,
//! here, rather than duplicated between prose a model reads and a schema
//! the harness validates against.

use crate::{
    harness::HandoffSchema,
    persona::{Persona, PersonaId, PersonaRegistry},
    pipeline::{
        AgentRole, ArchitectureReviewerHandoff, BuilderHandoff, CodeReviewerHandoff,
        CommitComplete, FinalCodeReviewerHandoff, IssueAnalysis, ProjectManagerHandoff,
        PullRequestReady, ResearchComplete, SoftwareArchitectHandoff, SubmitTests,
        TestReviewerHandoff,
    },
};

/// The persona id `AgentRole`'s own prompt and tool configuration already
/// live under -- the same embedded roster milestone 3 phase 16 and
/// milestone 4 phase 19 shipped.
pub fn persona_id_for(role: AgentRole) -> PersonaId {
    PersonaId::new(match role {
        AgentRole::IssueAnalyst => "issue-analyst",
        AgentRole::CodebaseResearcher => "codebase-researcher",
        AgentRole::SoftwareArchitect => "software-architect",
        AgentRole::ArchitectureReviewer => "architecture-reviewer",
        AgentRole::ProjectManager => "project-manager",
        AgentRole::TestEngineer => "test-engineer",
        AgentRole::TestReviewer => "test-reviewer",
        AgentRole::Builder => "builder",
        AgentRole::CodeReviewer => "code-reviewer",
        AgentRole::FinalCodeReviewer => "final-code-reviewer",
        AgentRole::CommitCrafter => "commit-crafter",
        AgentRole::PrAuthor => "pr-author",
    })
}

/// Looks `role`'s own persona up in `registry` and returns it with its
/// handoff schema attached, ready for [`crate::harness::Harness::run_turn`].
///
/// `None` when the operator's own configuration never resolved that
/// persona (an embedded default with no provider credentials configured
/// for it, say) -- the same "a convenience nobody opted into" reasoning
/// `PersonaRegistry::load` already applies elsewhere, not a bug to unwrap
/// past.
pub fn persona_for(role: AgentRole, registry: &PersonaRegistry) -> Option<Persona> {
    let persona = registry.get(&persona_id_for(role))?.clone();
    Some(Persona {
        handoff: Some(handoff_schema_for(role)),
        ..persona
    })
}

/// The handoff schema `AgentRole`'s persona must be run with -- see the
/// agent dispatcher (a later commit) for where this is actually attached
/// to a `Persona` before a turn runs.
///
/// Every description is written for the role holding it, not a generic
/// "finish the turn" instruction -- see [`HandoffSchema::description`]'s
/// own doc comment for why a generic description would hurt every
/// persona that used one.
pub fn handoff_schema_for(role: AgentRole) -> HandoffSchema {
    match role {
        AgentRole::IssueAnalyst => HandoffSchema::new(
            "Call this exactly once to finish your analysis: classify the issue, report on any \
             reproduction attempt, and recommend whether to proceed, ask for more information, or \
             skip it entirely.",
            schemars::schema_for!(IssueAnalysis).to_value(),
        ),
        AgentRole::CodebaseResearcher => HandoffSchema::new(
            "Call this exactly once you understand what the plan needs to know about this \
             repository: a summary of what you found and which files are relevant.",
            schemars::schema_for!(ResearchComplete).to_value(),
        ),
        AgentRole::SoftwareArchitect => HandoffSchema::new(
            "Call this to submit a completed plan, or to ask a clarifying question you cannot \
             resolve on your own before one can be written.",
            schemars::schema_for!(SoftwareArchitectHandoff).to_value(),
        ),
        AgentRole::ArchitectureReviewer => HandoffSchema::new(
            "Call this to approve the plan -- naming what it does well, not only what to fix -- \
             or to send it back with specific, actionable feedback.",
            schemars::schema_for!(ArchitectureReviewerHandoff).to_value(),
        ),
        AgentRole::ProjectManager => HandoffSchema::new(
            "Call this to start the next subtask's tests, or, once every subtask is committed, to \
             begin the final review of the whole project.",
            schemars::schema_for!(ProjectManagerHandoff).to_value(),
        ),
        AgentRole::TestEngineer => HandoffSchema::new(
            "Call this once your tests are written and you've confirmed they fail for the right \
             reason, before any implementation exists.",
            schemars::schema_for!(SubmitTests).to_value(),
        ),
        AgentRole::TestReviewer => HandoffSchema::new(
            "Call this to approve the tests as a specification, or to send them back with what \
             must change.",
            schemars::schema_for!(TestReviewerHandoff).to_value(),
        ),
        AgentRole::Builder => HandoffSchema::new(
            "Call this once you've implemented the subtask against its approved tests, or to ask \
             a question you cannot resolve on your own.",
            schemars::schema_for!(BuilderHandoff).to_value(),
        ),
        AgentRole::CodeReviewer => HandoffSchema::new(
            "Call this to approve this subtask's implementation, or to send it back with what \
             must change.",
            schemars::schema_for!(CodeReviewerHandoff).to_value(),
        ),
        AgentRole::FinalCodeReviewer => HandoffSchema::new(
            "Call this to approve the whole project together, or to send one subtask back for \
             changes if it doesn't hold up against everything else that was built.",
            schemars::schema_for!(FinalCodeReviewerHandoff).to_value(),
        ),
        AgentRole::CommitCrafter => HandoffSchema::new(
            "Call this once you've committed the approved changes for this subtask.",
            schemars::schema_for!(CommitComplete).to_value(),
        ),
        AgentRole::PrAuthor => HandoffSchema::new(
            "Call this once you've written the pull request's title and body from the real diff \
             and commit history.",
            schemars::schema_for!(PullRequestReady).to_value(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{persona::AiConfig, provider::ProviderRegistry, types::ModelRef};

    const EVERY_ROLE: [AgentRole; 12] = [
        AgentRole::IssueAnalyst,
        AgentRole::CodebaseResearcher,
        AgentRole::SoftwareArchitect,
        AgentRole::ArchitectureReviewer,
        AgentRole::ProjectManager,
        AgentRole::TestEngineer,
        AgentRole::TestReviewer,
        AgentRole::Builder,
        AgentRole::CodeReviewer,
        AgentRole::FinalCodeReviewer,
        AgentRole::CommitCrafter,
        AgentRole::PrAuthor,
    ];

    #[test]
    fn test_every_role_uses_the_conventional_handoff_tool_name() {
        for role in EVERY_ROLE {
            assert_eq!(handoff_schema_for(role).tool_name, "handoff");
        }
    }

    #[test]
    fn test_every_role_has_its_own_non_empty_description() {
        for role in EVERY_ROLE {
            assert!(
                !handoff_schema_for(role).description.is_empty(),
                "{role:?} should have a real description, not a generic placeholder"
            );
        }
    }

    #[test]
    fn test_no_two_roles_share_the_exact_same_description() {
        // a cheap guard against copy-pasting one role's description onto
        // another and forgetting to actually write its own
        let descriptions: Vec<String> = EVERY_ROLE
            .iter()
            .map(|role| handoff_schema_for(*role).description)
            .collect();

        for (i, a) in descriptions.iter().enumerate() {
            for (j, b) in descriptions.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "role {i} and role {j} share a description");
                }
            }
        }
    }

    #[test]
    fn test_issue_analyst_schema_is_an_object() {
        assert_eq!(
            handoff_schema_for(AgentRole::IssueAnalyst).schema["type"],
            "object"
        );
    }

    #[test]
    fn test_software_architect_schema_is_one_of_its_two_actions() {
        assert!(
            handoff_schema_for(AgentRole::SoftwareArchitect).schema["oneOf"].is_array(),
            "a role with more than one possible action should schema as oneOf them"
        );
    }

    #[test]
    fn test_project_manager_schema_is_one_of_its_two_actions() {
        assert!(handoff_schema_for(AgentRole::ProjectManager).schema["oneOf"].is_array());
    }

    #[test]
    fn test_persona_id_for_every_role_matches_the_embedded_roster() {
        let mut config = AiConfig::default();
        config.default_model = Some(ModelRef::new("anthropic", "claude-opus-5"));
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        for role in EVERY_ROLE {
            assert!(
                registry.get(&persona_id_for(role)).is_some(),
                "{role:?} should map to a persona id the embedded roster actually resolves"
            );
        }
    }

    #[test]
    fn test_persona_for_attaches_the_matching_handoff_schema() {
        let mut config = AiConfig::default();
        config.default_model = Some(ModelRef::new("anthropic", "claude-opus-5"));
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        let persona =
            persona_for(AgentRole::IssueAnalyst, &registry).expect("should resolve a persona");
        assert_eq!(persona.id, persona_id_for(AgentRole::IssueAnalyst));
        assert_eq!(
            persona.handoff.expect("should carry a handoff").description,
            handoff_schema_for(AgentRole::IssueAnalyst).description
        );
    }

    #[test]
    fn test_persona_for_keeps_the_base_personas_own_tools_and_sandbox_policy() {
        let mut config = AiConfig::default();
        config.default_model = Some(ModelRef::new("anthropic", "claude-opus-5"));
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        let base = registry
            .get(&persona_id_for(AgentRole::Builder))
            .expect("should resolve")
            .clone();
        let dispatched = persona_for(AgentRole::Builder, &registry).expect("should resolve");

        assert_eq!(dispatched.sandbox, base.sandbox);
        assert_eq!(dispatched.budget, base.budget);
    }

    #[test]
    fn test_persona_for_returns_none_when_the_registry_never_resolved_it() {
        // no default_model at all -- the embedded roster resolves nothing
        let config = AiConfig::default();
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let registry = PersonaRegistry::load(&config, &providers).expect("should resolve");

        assert!(persona_for(AgentRole::IssueAnalyst, &registry).is_none());
    }
}
