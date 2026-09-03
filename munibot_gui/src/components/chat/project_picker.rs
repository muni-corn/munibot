use dioxus::prelude::*;
use munibot_api::server_fns::{
    chat::conversation::get_conversation,
    project::{list_projects_for_picker, set_conversation_project},
};

/// Parses the picker `<select>`'s own value into what
/// `set_conversation_project` expects: the empty string (the "no
/// project" option's own value) unbinds, anything else is a project id.
fn parse_project_selection(value: &str) -> Option<i64> {
    if value.is_empty() {
        None
    } else {
        value.parse::<i64>().ok()
    }
}

/// A dropdown that binds or unbinds a conversation to a registered
/// project, labelling each option with its image status - the picker
/// beside the persona picker the plan for this milestone describes.
///
/// Binding takes effect starting with this conversation's *next* turn: a
/// `coder` persona's sandbox then provisions over that project's own
/// worktree instead of a fresh scratch one every time, see
/// `munibot_ai::service::Ai::resolve_workspace_override`. Nothing about
/// an already-running turn changes retroactively.
#[component]
pub fn ProjectPicker(conversation_id: i64) -> Element {
    let mut conversation =
        use_resource(move || async move { get_conversation(conversation_id).await });
    let projects = use_resource(list_projects_for_picker);
    let mut error = use_signal(|| None::<String>);

    let current_project_id = match &*conversation.read() {
        Some(Ok(summary)) => summary.project_id,
        _ => None,
    };

    let on_change = move |event: Event<FormData>| {
        let project_id = parse_project_selection(&event.value());

        spawn(async move {
            match set_conversation_project(conversation_id, project_id).await {
                Ok(()) => {
                    error.set(None);
                    conversation.restart();
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let content = match &*projects.read() {
        Some(Ok(list)) if list.is_empty() => rsx! {
            span { class: "text-xs text-slate-500", "no projects registered yet" }
        },
        Some(Ok(list)) => rsx! {
            select {
                class: "select w-56 select-sm",
                value: current_project_id.map(|id| id.to_string()).unwrap_or_default(),
                onchange: on_change,
                option { value: "", "no project" }
                for project in list.iter() {
                    option { key: "{project.id}", value: "{project.id}",
                        "{project.display_name} ({project.image_status})"
                    }
                }
            }
        },
        Some(Err(_)) => rsx! {
            span { class: "text-xs text-error", "couldn't load projects." }
        },
        None => rsx! {
            span { class: "text-xs text-slate-400", "loading projects..." }
        },
    };

    rsx! {
        div { class: "flex items-center gap-2",
            i { class: "text-slate-400 ph-duotone ph-folder-simple" }
            {content}
            if let Some(message) = &*error.read() {
                span { class: "text-xs text-error", {message.as_str()} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_empty_value_is_the_no_project_option_and_unbinds() {
        assert_eq!(parse_project_selection(""), None);
    }

    #[test]
    fn test_a_numeric_value_selects_that_project() {
        assert_eq!(parse_project_selection("42"), Some(42));
    }

    #[test]
    fn test_a_malformed_value_is_treated_as_no_selection_rather_than_panicking() {
        assert_eq!(parse_project_selection("not-a-number"), None);
    }
}
