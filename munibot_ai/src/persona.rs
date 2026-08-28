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

pub use config::{AiConfig, BudgetConfig, PersonaConfig};
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
    fn test_coder_prompt_states_it_cannot_run_or_modify_code_yet() {
        // the plan's explicit requirement for this persona in this milestone
        let source = include_str!("../prompts/coder.md");
        assert!(
            source.contains("cannot run it") || source.contains("no ability to execute"),
            "the coder prompt must state plainly that it cannot execute or modify code yet"
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

    #[test]
    fn test_test_engineer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/test-engineer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_test_engineer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/test-engineer.md"));
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
    fn test_test_engineer_prompt_names_its_real_tools() {
        let source = include_str!("../prompts/test-engineer.md");
        for tool in ["`read`", "`write`", "`edit`", "`bash`", "`grep`", "`glob`"] {
            assert!(
                source.contains(tool),
                "the test engineer prompt should name its real tool {tool:?}"
            );
        }
    }

    #[test]
    fn test_test_engineer_prompt_instructs_actually_running_the_tests() {
        // milestone 4's own explicit divergence from the municode source,
        // which could only write tests without ever running them - see
        // milestone-4-sandbox.md commit 148's own description
        let source = include_str!("../prompts/test-engineer.md");
        assert!(
            source.contains("Run what you wrote") || source.contains("actually run"),
            "the test engineer prompt must instruct actually running the tests it writes, not \
             just describing them"
        );
        assert!(
            source.contains("fail") && source.contains("right reason"),
            "the test engineer prompt should instruct confirming tests fail for the right reason \
             before an implementation exists"
        );
    }

    #[test]
    fn test_test_engineer_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/test-engineer.md");
        for pipeline_only in [
            "SubmitTests",
            "RequestBuildHelp",
            "```json",
            "start-task-tests",
            "tests_written",
            "all_passing",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the test engineer prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_final_code_reviewer_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/final-code-reviewer.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_final_code_reviewer_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/final-code-reviewer.md"));
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
    fn test_final_code_reviewer_prompt_names_its_real_read_only_tools() {
        let source = include_str!("../prompts/final-code-reviewer.md");
        for tool in ["`read`", "`grep`", "`glob`", "`bash`"] {
            assert!(
                source.contains(tool),
                "the final code reviewer prompt should name its real tool {tool:?}"
            );
        }
        assert!(
            !source.contains("`write`") && !source.contains("`edit`"),
            "a reviewer should never be described as having write/edit access"
        );
    }

    #[test]
    fn test_final_code_reviewer_prompt_states_the_merge_ready_bar() {
        // the plan's own framing: "ready to merge as a pull request," not
        // "theoretically perfect"
        let source = include_str!("../prompts/final-code-reviewer.md");
        assert!(source.contains("ready to merge"));
    }

    #[test]
    fn test_final_code_reviewer_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/final-code-reviewer.md");
        for pipeline_only in [
            "ProjectComplete",
            "RequestCodeChanges",
            "```json",
            "all-changes",
            "completed-tasks",
            "files_reviewed",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the final code reviewer prompt should not contain pipeline-only text \
                 {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_commit_crafter_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/commit-crafter.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_commit_crafter_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/commit-crafter.md"));
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
    fn test_commit_crafter_prompt_names_its_real_tools() {
        let source = include_str!("../prompts/commit-crafter.md");
        for tool in ["`bash`", "`read`", "`grep`", "`glob`"] {
            assert!(
                source.contains(tool),
                "the commit crafter prompt should name its real tool {tool:?}"
            );
        }
    }

    #[test]
    fn test_commit_crafter_prompt_forbids_bulk_staging_and_amending() {
        let source = include_str!("../prompts/commit-crafter.md");
        assert!(source.contains("git add ."));
        assert!(source.to_lowercase().contains("never amend"));
    }

    #[test]
    fn test_commit_crafter_prompt_points_at_the_target_repositories_own_conventions() {
        // rather than hardcoding this repository's own commit style into a
        // generic persona - see docs/plans/ai/milestone-4-sandbox.md's own
        // note on why
        let source = include_str!("../prompts/commit-crafter.md");
        assert!(
            source.contains("AGENTS.md") || source.contains("CONTRIBUTING.md"),
            "the commit crafter prompt should point at the target repository's own documented \
             conventions"
        );
    }

    #[test]
    fn test_commit_crafter_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/commit-crafter.md");
        for pipeline_only in [
            "CommitComplete",
            "```json",
            "commit-message",
            "code-changes",
            "files_committed",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the commit crafter prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }

    #[test]
    fn test_pr_author_prompt_declares_exactly_user_name_and_platform() {
        let template = PromptTemplate::new(include_str!("../prompts/pr-author.md"));
        let mut variables = template.required_variables();
        variables.sort();
        assert_eq!(variables, vec![
            "platform".to_string(),
            "user_name".to_string()
        ]);
    }

    #[test]
    fn test_pr_author_prompt_renders_with_a_sample_context() {
        let template = PromptTemplate::new(include_str!("../prompts/pr-author.md"));
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
    fn test_pr_author_prompt_names_its_real_read_only_tools() {
        let source = include_str!("../prompts/pr-author.md");
        for tool in ["`read`", "`grep`", "`glob`", "`bash`"] {
            assert!(
                source.contains(tool),
                "the pr author prompt should name its real tool {tool:?}"
            );
        }
    }

    #[test]
    fn test_pr_author_prompt_describes_the_required_body_sections() {
        let source = include_str!("../prompts/pr-author.md");
        for section in ["Summary", "Changes", "Files changed", "Testing"] {
            assert!(
                source.contains(section),
                "the pr author prompt should describe the {section:?} section"
            );
        }
    }

    #[test]
    fn test_pr_author_prompt_has_no_pipeline_output_contract() {
        let source = include_str!("../prompts/pr-author.md");
        for pipeline_only in [
            "PullRequestReady",
            "```json",
            "all-changes",
            "completed-tasks",
        ] {
            assert!(
                !source.contains(pipeline_only),
                "the pr author prompt should not contain pipeline-only text {pipeline_only:?}"
            );
        }
    }
}
