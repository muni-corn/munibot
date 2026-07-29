use std::collections::{HashMap, HashSet};

use crate::types::AiError;

/// A prompt template with `{{variable}}` placeholders, substituted at render
/// time.
///
/// Deliberately minimal: no conditionals, loops, or filters, just named
/// substitution. A persona's prompt is markdown prose with a handful of named
/// holes (`{{user_name}}`, `{{memories}}`), not a general templating problem,
/// so a hand-rolled scanner is enough and avoids a templating-engine dependency
/// for something this small.
#[derive(Clone, Debug, PartialEq)]
pub struct PromptTemplate {
    source: String,
}

impl PromptTemplate {
    /// Builds a template from its raw source text.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }

    /// Every `{{variable}}` name this template references, deduplicated, in
    /// first-occurrence order.
    pub fn required_variables(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut ordered = Vec::new();
        for name in Self::variable_occurrences(&self.source) {
            if seen.insert(name.clone()) {
                ordered.push(name);
            }
        }
        ordered
    }

    /// Renders the template, substituting every `{{variable}}` from `context`.
    ///
    /// Fails only when the template references a variable `context` does not
    /// provide, and names every missing one at once rather than stopping at
    /// the first, so a caller fixes its context in one pass instead of one
    /// variable at a time. A `context` entry the template never references
    /// is silently ignored.
    pub fn render(&self, context: &HashMap<String, String>) -> Result<String, AiError> {
        let missing: Vec<String> = self
            .required_variables()
            .into_iter()
            .filter(|name| !context.contains_key(name))
            .collect();

        if !missing.is_empty() {
            return Err(AiError::Config(format!(
                "this prompt template is missing: {} :<",
                missing.join(", ")
            )));
        }

        let mut rendered = String::with_capacity(self.source.len());
        let mut remaining = self.source.as_str();

        while let Some(start) = remaining.find("{{") {
            rendered.push_str(&remaining[..start]);
            let after_open = &remaining[start + 2..];

            match after_open.find("}}") {
                Some(end) => {
                    let name = after_open[..end].trim();
                    // every name this loop can produce was already checked against context
                    // above, via the identical scan in required_variables()
                    if let Some(value) = context.get(name) {
                        rendered.push_str(value);
                    }
                    remaining = &after_open[end + 2..];
                }
                None => {
                    // an unterminated `{{` - keep it as literal text rather than silently
                    // dropping the rest of the template
                    rendered.push_str("{{");
                    remaining = after_open;
                }
            }
        }
        rendered.push_str(remaining);

        Ok(rendered)
    }

    fn variable_occurrences(source: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut remaining = source;

        while let Some(start) = remaining.find("{{") {
            let after_open = &remaining[start + 2..];
            match after_open.find("}}") {
                Some(end) => {
                    names.push(after_open[..end].trim().to_string());
                    remaining = &after_open[end + 2..];
                }
                None => break,
            }
        }

        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn test_template_with_no_variables_renders_unchanged() {
        let template = PromptTemplate::new("just plain text");
        assert_eq!(template.render(&ctx(&[])).unwrap(), "just plain text");
    }

    #[test]
    fn test_single_variable_is_substituted() {
        let template = PromptTemplate::new("hello, {{name}}!");
        assert_eq!(
            template.render(&ctx(&[("name", "muni")])).unwrap(),
            "hello, muni!"
        );
    }

    #[test]
    fn test_multiple_variables_are_all_substituted() {
        let template = PromptTemplate::new("{{greeting}}, {{name}}!");
        let rendered = template
            .render(&ctx(&[("greeting", "hi"), ("name", "muni")]))
            .unwrap();
        assert_eq!(rendered, "hi, muni!");
    }

    #[test]
    fn test_a_repeated_variable_is_substituted_everywhere_it_appears() {
        let template = PromptTemplate::new("{{name}} and {{name}} again");
        assert_eq!(
            template.render(&ctx(&[("name", "muni")])).unwrap(),
            "muni and muni again"
        );
    }

    #[test]
    fn test_required_variables_deduplicates_and_preserves_first_occurrence_order() {
        let template = PromptTemplate::new("{{b}} then {{a}} then {{b}} again");
        assert_eq!(template.required_variables(), vec![
            "b".to_string(),
            "a".to_string()
        ]);
    }

    #[test]
    fn test_whitespace_inside_braces_is_trimmed() {
        let template = PromptTemplate::new("{{ name }}");
        assert_eq!(template.required_variables(), vec!["name".to_string()]);
        assert_eq!(template.render(&ctx(&[("name", "muni")])).unwrap(), "muni");
    }

    #[test]
    fn test_missing_variable_is_an_error() {
        let template = PromptTemplate::new("hello, {{name}}");
        let error = template
            .render(&ctx(&[]))
            .expect_err("should fail without name");
        assert!(error.to_string().contains("name"));
    }

    #[test]
    fn test_every_missing_variable_is_named_in_one_error() {
        let template = PromptTemplate::new("{{a}} {{b}} {{c}}");
        let error = template
            .render(&ctx(&[("b", "present")]))
            .expect_err("should fail");
        let message = error.to_string();

        assert!(
            message.contains('a'),
            "the error should name a: {message:?}"
        );
        assert!(
            message.contains('c'),
            "the error should name c: {message:?}"
        );
        assert!(
            !message.contains('b'),
            "b was provided and should not be listed as missing: {message:?}"
        );
    }

    #[test]
    fn test_unreferenced_context_entries_are_ignored() {
        let template = PromptTemplate::new("hello, {{name}}");
        let rendered = template
            .render(&ctx(&[("name", "muni"), ("unused", "whatever")]))
            .expect("extra context entries should not cause a failure");
        assert_eq!(rendered, "hello, muni");
    }

    #[test]
    fn test_unterminated_brace_is_kept_as_literal_text() {
        let template = PromptTemplate::new("here is a stray {{ oops");
        // no variable is required, since the brace never closes
        assert!(template.required_variables().is_empty());
        assert_eq!(
            template.render(&ctx(&[])).unwrap(),
            "here is a stray {{ oops"
        );
    }

    #[test]
    fn test_empty_template_renders_to_an_empty_string() {
        let template = PromptTemplate::new("");
        assert_eq!(template.render(&ctx(&[])).unwrap(), "");
    }
}
