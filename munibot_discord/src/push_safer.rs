use poise::serenity_prelude::*;

// A trait for implementing a "safer" version of serenity's `push_safe`,
// additionally removing Markdown link formatting.
pub trait PushSafer {
    fn push_safer(&mut self, content: impl Into<Content>) -> &mut Self;
}

impl PushSafer for MessageBuilder {
    fn push_safer(&mut self, content: impl Into<Content>) -> &mut Self {
        let content = escape_link_boundaries(content);
        self.push_safe(content);

        self
    }
}

fn escape_link_boundaries(content: impl Into<Content>) -> String {
    // replace closing parentheses is unnecessary, for some reason
    content
        .into()
        .to_string()
        .replace("[", "\\[")
        .replace("]", "\\]")
        .replace("(", "\\(")
        .replace(".", "\u{2024}")
}
