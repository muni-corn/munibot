//! Persona configuration: what makes a companion different from a researcher.
//!
//! A persona is a model, a system prompt, a tool allowlist, a budget, and an
//! optional handoff schema - the same type serves a casual chat persona and a
//! pipeline agent role alike. This is the first module in this crate that
//! depends on `munibot_core`, since [`AiConfig`] is loaded from the same
//! configuration file `munibot_core::Config` reads (see its own doc comment for
//! why it is a separate, independent load rather than a shared field).

pub mod config;
pub mod output_filter;
pub mod registry;
pub mod template;
pub mod types;

pub use config::{
    AiConfig, BudgetConfig, CrisisResourceConfig, PersonaConfig, RateLimitConfig,
    RateLimitPolicyConfig, SpendCapConfig, SpendCapPolicyConfig,
};
pub use output_filter::{OutputLimits, filter_output};
pub use registry::PersonaRegistry;
pub use template::PromptTemplate;
pub use types::{MemoryPolicy, Persona, PersonaId, SandboxPolicy};

/// Verifies each embedded persona prompt against expectations that would
/// otherwise only surface once the persona registry (a later commit) actually
/// loads them - broken template syntax or an unexpected variable set should
/// fail here, immediately, rather than waiting for that.
#[cfg(test)]
mod prompt_tests {
    use super::PromptTemplate;

    #[test]
    fn test_companion_prompt_declares_exactly_user_name_platform_and_memories() {
        let template = PromptTemplate::new(include_str!("../prompts/companion.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "memories".to_string(),
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_companion_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/companion.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
            ("memories".to_string(), "Nothing recorded yet.".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template
            .render(&context)
            .expect("should render with every variable provided");

        assert!(
            rendered.contains("muni"),
            "the user's name should appear in the rendered prompt"
        );
        assert!(rendered.contains("Discord"));
        assert!(rendered.contains("Nothing recorded yet."));
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_companion_prompt_is_not_empty() {
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.len() > 500,
            "the companion prompt should be a real, substantial prompt"
        );
    }

    #[test]
    fn test_companion_prompt_states_it_can_search_and_fetch_the_web() {
        // milestone 2 phase 13's own requirement: the companion carries research
        // tools himself, so the prompt must say so for both of them, not just
        // searching - a link handed to him or turned up by a search should
        // actually get read, not guessed at from its url alone
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("search the web"),
            "the companion prompt must state plainly that it can search the web"
        );
        assert!(
            source.contains("follow a specific link") || source.contains("follow a link"),
            "the companion prompt must state plainly that it can fetch and read a specific link, \
             not just search"
        );
    }

    #[test]
    fn test_companion_prompt_distinguishes_same_conversation_recall_from_opt_in_memory() {
        // milestone 2 phase 13's own requirement: states plainly what it does and
        // does not remember. remembering what was just said needs no permission;
        // only carrying something into a *different*, later conversation does
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("earlier in _this_ conversation")
                || source.contains("this conversation"),
            "the companion prompt must state that recalling earlier in the same conversation \
             needs no permission or opt-in"
        );
        assert!(
            source.contains("opted into memory"),
            "the companion prompt must still state that only opted-in memory survives into a \
             different, later conversation"
        );
    }

    #[test]
    fn test_companion_prompt_distinguishes_platform_formatting() {
        // milestone 2 phase 13's own requirement: a much longer horizon than a
        // discord reply, and the web renders real markdown while discord doesn't
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("renders full markdown"),
            "the companion prompt must state that the web renders full markdown"
        );
        assert!(
            source.to_lowercase().contains("discord"),
            "the companion prompt must call out discord's narrower formatting by name"
        );
    }

    #[test]
    fn test_companion_prompt_still_renders_after_the_delegation_section() {
        // the delegation section (milestone 3 phase 15) references
        // {{user_name}} again inside its own prose - a stray literal
        // {{...}} left in by mistake would fail this the same way a typo'd
        // variable name would
        let template = PromptTemplate::new(include_str!("../prompts/companion.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
            ("memories".to_string(), "Nothing recorded yet.".to_string()),
        ]
        .into_iter()
        .collect();

        template
            .render(&context)
            .expect("should still render with every variable provided");
    }

    #[test]
    fn test_companion_prompt_states_when_to_bring_in_a_specialist() {
        // milestone 3 phase 15's own requirement: delegate when asked, or
        // when a task plainly exceeds conversation - never reflexively
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("bring in a specialist") || source.contains("bring in"),
            "the companion prompt must state that it can bring in a specialist"
        );
        assert!(
            source.to_lowercase().contains("not reflexively"),
            "the companion prompt must state that delegation is not the reflexive choice for most \
             questions"
        );
    }

    #[test]
    fn test_companion_prompt_requires_saying_delegation_out_loud() {
        // milestone 3 phase 15's own requirement: never silently hand off
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("say so plainly"),
            "the companion prompt must require saying out loud that a specialist was brought in"
        );
    }

    #[test]
    fn test_companion_prompt_forbids_presenting_specialist_work_as_its_own() {
        // milestone 3 phase 15's own requirement: report back in his own
        // voice, never present a specialist's work as his own. checked as
        // two short substrings rather than one long phrase, since the
        // prompt's own prose wraps across lines and a raw file read keeps
        // the literal newline
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("present their work as your own"),
            "the companion prompt must forbid presenting a specialist's work as its own"
        );
    }

    #[test]
    fn test_companion_prompt_states_a_brief_must_stand_alone() {
        // milestone 3 phase 15 commit 113's own requirement: the specialist
        // never sees the conversation, only the written brief
        let source = include_str!("../prompts/companion.md");
        assert!(
            source.contains("self-contained brief")
                || source.contains("never see this conversation"),
            "the companion prompt must state that a specialist never sees the conversation and \
             needs a self-contained brief"
        );
    }

    #[test]
    fn test_writer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/writer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_writer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/writer.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_researcher_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/researcher.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_researcher_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/researcher.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_researcher_prompt_states_the_citation_requirement() {
        // the one rule the plan explicitly requires of this persona
        let source = include_str!("../prompts/researcher.md");
        assert!(
            source.contains("traceable to a source"),
            "the researcher prompt must state the mandatory-citation rule explicitly"
        );
    }

    #[test]
    fn test_coder_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/coder.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_coder_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/coder.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_coder_prompt_states_it_can_run_and_verify_code() {
        // milestone 4 gives the coder sandbox tools - the prompt must say so,
        // and push toward actually running things rather than asserting
        // correctness (the inverse of this test's own pre-milestone-4 name)
        let source = include_str!("../prompts/coder.md");
        assert!(
            source.contains("bash") && source.contains("sandbox"),
            "the coder prompt must mention it has sandbox tools available"
        );
        assert!(
            source.contains("run the tests") || source.contains("run it"),
            "the coder prompt must instruct running code to verify it, not just asserting \
             correctness"
        );
    }

    #[test]
    fn test_software_architect_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/software-architect.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_software_architect_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/software-architect.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_software_architect_prompt_keeps_the_planning_principles() {
        // milestone 3 phase 16's porting rule: role, standards, and judgement
        // stay in the markdown even after the output contract is stripped
        let source = include_str!("../prompts/software-architect.md");
        for principle in ["Atomicity", "Completeness", "Ordering", "Consistency"] {
            assert!(
                source.contains(principle),
                "the software architect prompt must keep the {principle} planning principle"
            );
        }
    }

    #[test]
    fn test_issue_analyst_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/issue-analyst.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_issue_analyst_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/issue-analyst.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_issue_analyst_prompt_states_it_has_no_sandbox_yet() {
        // milestone 3 phase 16's own requirement: this role lands before the
        // sandbox does (milestone 4), so it must not imply it can actually
        // run anything to reproduce a bug
        let source = include_str!("../prompts/issue-analyst.md");
        assert!(
            source.contains("no sandbox yet"),
            "the issue analyst prompt must state plainly that it has no sandbox yet"
        );
    }

    #[test]
    fn test_issue_analyst_prompt_has_no_pipeline_output_contract() {
        // the porting rule's whole point - see software-architect's own
        // equivalent test - plus the sandboxed tools (Read/Grep/Glob/Bash)
        // this role's municode source assumed but does not have yet
        let source = include_str!("../prompts/issue-analyst.md");
        for pipeline_only in [
            "IssueAnalysis",
            "RequestAnalysisHelp",
            "Handoff",
            "```markdown",
            "github-issue",
            "`Bash`",
            "`Grep`",
            "`Glob`",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the issue analyst prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_code_reviewer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/code-reviewer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_code_reviewer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/code-reviewer.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_code_reviewer_prompt_reviews_pasted_code_as_readily_as_a_diff() {
        // milestone 3 phase 16's own requirement for this persona. checked as
        // short substrings rather than one long phrase, since the prompt's
        // own prose wraps across lines and a raw file read keeps the
        // literal newline
        let source = include_str!("../prompts/code-reviewer.md");
        assert!(source.contains("pasted snippet"));
        assert!(source.contains("no repository behind it"));
    }

    #[test]
    fn test_code_reviewer_prompt_calibrates_severity() {
        let source = include_str!("../prompts/code-reviewer.md");
        for severity in ["Critical", "Major", "Minor", "Nit"] {
            assert!(
                source.contains(severity),
                "the code reviewer prompt must keep the {severity} severity level"
            );
        }
    }

    #[test]
    fn test_code_reviewer_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/code-reviewer.md");
        for pipeline_only in [
            "ApproveCode",
            "RequestCodeChanges",
            "```json",
            "task-spec",
            "codebase-summary",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the code reviewer prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_test_reviewer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/test-reviewer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_test_reviewer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/test-reviewer.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_test_reviewer_prompt_reviews_pasted_tests_as_readily_as_a_diff() {
        // milestone 3 phase 16's own requirement for this persona
        let source = include_str!("../prompts/test-reviewer.md");
        assert!(
            source.contains("no filesystem"),
            "the test reviewer prompt must state it has no filesystem to go looking for more \
             context, reviewing exactly what it's shown"
        );
    }

    #[test]
    fn test_test_reviewer_prompt_keeps_the_precision_judgement() {
        // the single most important judgement call in this role: would a
        // correct implementation pass, would a subtly wrong one fail
        let source = include_str!("../prompts/test-reviewer.md");
        assert!(
            source.contains("Precision"),
            "the test reviewer prompt must keep the precision judgement criterion"
        );
    }

    #[test]
    fn test_test_reviewer_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/test-reviewer.md");
        for pipeline_only in [
            "ApproveTests",
            "RequestTestChanges",
            "```json",
            "start-task-tests",
            "codebase-summary",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the test reviewer prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_architecture_reviewer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/architecture-reviewer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_architecture_reviewer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/architecture-reviewer.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_architecture_reviewer_prompt_keeps_all_seven_review_criteria() {
        let source = include_str!("../prompts/architecture-reviewer.md");
        for criterion in [
            "Completeness",
            "Correctness",
            "Atomicity",
            "Ordering",
            "Consistency",
            "Feasibility",
            "Instruction quality",
        ] {
            assert!(
                source.contains(criterion),
                "the architecture reviewer prompt must keep the {criterion} review criterion"
            );
        }
    }

    #[test]
    fn test_architecture_reviewer_prompt_never_reintroduces_the_stray_shell_command() {
        // docs/plans/ai/overview.md documents this exact defect in the
        // source municode prompt - a `git config` command spliced into a
        // sentence - as one to fix during the port, not inherit
        let source = include_str!("../prompts/architecture-reviewer.md");
        assert!(
            !source.contains("git config"),
            "the architecture reviewer prompt must not carry over municode's stray shell command"
        );
    }

    #[test]
    fn test_architecture_reviewer_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/architecture-reviewer.md");
        for pipeline_only in [
            "ApprovePlan",
            "RequestPlanChanges",
            "```json",
            "proposed-plan",
            "codebase-summary",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the architecture reviewer prompt should not contain pipeline-only text \
                 {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_project_manager_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/project-manager.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_project_manager_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/project-manager.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_project_manager_prompt_never_reintroduces_the_starttask_naming_drift() {
        // docs/plans/ai/overview.md documents this exact defect in the
        // source municode prompt - StartTask vs StartTaskTests naming drift
        // - as one to fix during the port; stripping the whole pipeline
        // action-name vocabulary removes it by construction
        let source = include_str!("../prompts/project-manager.md");
        assert!(!source.contains("StartTask"));
    }

    #[test]
    fn test_project_manager_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/project-manager.md");
        for pipeline_only in [
            "BeginFinalReview",
            "```json",
            "completed-tasks",
            "task-reviews",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the project manager prompt should not contain pipeline-only text \
                 {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_software_architect_prompt_has_no_pipeline_output_contract() {
        // the porting rule's whole point: output-contract prose ("return
        // JSON shaped like...") belongs in a future HandoffSchema, not the
        // markdown - none of municode's own JSON action-block vocabulary
        // should have survived the port
        let source = include_str!("../prompts/software-architect.md");
        for pipeline_only in [
            "RequestPlanHelp",
            "CreatePlan",
            "```json",
            "codebase-summary",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the software architect prompt should not contain pipeline-only text \
                 {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_critic_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/critic.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_critic_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/critic.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_critic_prompt_states_its_three_part_structure() {
        // the plan's own words for this persona: "structured as what works,
        // what does not, and what to try next"
        let source = include_str!("../prompts/critic.md");
        assert!(source.contains("What works"));
        assert!(source.contains("What doesn't"));
        assert!(source.contains("What to try next"));
    }

    #[test]
    fn test_critic_prompt_asks_for_specificity_over_vague_praise() {
        // the plan's own reasoning for this persona: "vague praise is
        // worthless to someone trying to improve"
        let source = include_str!("../prompts/critic.md");
        assert!(
            source.to_lowercase().contains("vague"),
            "the critic prompt should explicitly reject vague feedback, not just imply it"
        );
    }

    #[test]
    fn test_codebase_researcher_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/codebase-researcher.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_codebase_researcher_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/codebase-researcher.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_codebase_researcher_prompt_names_its_real_read_only_tools() {
        // milestone 4's own wrinkle for the hands-on roles: unlike the
        // milestone-3 roster, these actually have real tool access, and
        // should say so in present tense - see docs/notes/persona-prompt-
        // porting.md's note on this
        let source = include_str!("../prompts/codebase-researcher.md");
        assert!(source.contains("`read`"));
        assert!(source.contains("`grep`"));
        assert!(source.contains("`glob`"));
    }

    #[test]
    fn test_codebase_researcher_prompt_states_it_does_not_modify_anything() {
        let source = include_str!("../prompts/codebase-researcher.md");
        assert!(
            source.contains("do not implement") || source.contains("do not have `write`"),
            "the codebase researcher prompt must state plainly that it does not modify files"
        );
    }

    #[test]
    fn test_codebase_researcher_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/codebase-researcher.md");
        for pipeline_only in [
            "ResearchComplete",
            "```json",
            "user-instructions",
            "relevant_files",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the codebase researcher prompt should not contain pipeline-only text \
                 {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_builder_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/builder.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_builder_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/builder.md"));
        let context = [
            ("user_name".to_string(), "muni".to_string()),
            ("platform".to_string(), "Discord".to_string()),
        ]
        .into_iter()
        .collect();

        let rendered = template.render(&context).expect("should render");
        assert!(
            !rendered.contains("{{"),
            "no placeholder should survive rendering"
        );
    }

    #[test]
    fn test_builder_prompt_names_its_real_tools() {
        let source = include_str!("../prompts/builder.md");
        for tool in ["`read`", "`write`", "`edit`", "`bash`", "`grep`", "`glob`"] {
            assert!(
                source.contains(tool),
                "the builder prompt should name its real tool {tool:?}"
            );
        }
    }

    #[test]
    fn test_builder_prompt_forbids_creating_commits() {
        // the plan's own scope boundary: "the Commit Crafter handles that"
        let source = include_str!("../prompts/builder.md");
        assert!(
            source.contains("Don't create git commits")
                || source.contains("don't create git commits")
        );
    }

    #[test]
    fn test_builder_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/builder.md");
        for pipeline_only in [
            "SubmitCode",
            "RequestBuildHelp",
            "```json",
            "start-task-tests",
            "reviewer-feedback",
            "files_changed",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the builder prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }
}
