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

pub use config::{AiConfig, BudgetConfig, CrisisResourceConfig, PersonaConfig};
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
}
