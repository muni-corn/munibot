//! Filters an assistant response for safe display on a platform surface.
//!
//! A model's raw text is never sent to a platform unfiltered: this module
//! strips anything that platform mention syntax would turn into an unintended
//! ping, caps the length to what the platform accepts, and normalizes unicode
//! confusables so nothing invisible or lookalike survives into what a user
//! reads.

use decancer::Options;

/// The number of user mentions above which they are collapsed into a single
/// note instead of being left in place. A handful of mentions in a response is
/// normal (quoting who said what); a flood of them is a mass-ping attempt.
const MASS_MENTION_THRESHOLD: usize = 3;

/// Platform-specific limits applied when filtering a response before it reaches
/// a user surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLimits {
    /// The maximum number of characters kept in the filtered text. Longer text
    /// is truncated and given a trailing ellipsis.
    pub max_length: usize,
}

impl OutputLimits {
    /// Builds a new set of limits with the given maximum length, in characters.
    pub fn new(max_length: usize) -> Self {
        Self { max_length }
    }
}

/// Filters `text` for safe display, applying `limits`.
///
/// In order: defuses `@everyone`/`@here`, strips Discord role-mention syntax,
/// collapses a flood of user mentions into a single note, runs the result
/// through `decancer` to normalize unicode confusables, then truncates to
/// `limits.max_length` with a trailing ellipsis. Truncation runs last,
/// after decancer, because decancer can *expand* some characters (its own
/// ellipsis handling turns a single `…` into three ASCII periods); enforcing
/// the length limit any earlier would not actually guarantee it in the final
/// output. Plain ASCII and accented Latin text keeps its original
/// capitalization; decancer does not track case for some multi-alphabet
/// confusables (fullwidth, fraktur, circled, and similar), so those are
/// lowercased regardless.
pub fn filter_output(text: &str, limits: OutputLimits) -> String {
    let defused = defuse_mass_pings(text);
    let mentions_filtered = filter_mentions(&defused);
    let cured = decancer_output(&mentions_filtered);
    truncate_with_ellipsis(&cured, limits.max_length)
}

/// Inserts a zero-width space right after the `@` in `@everyone`/`@here` so the
/// text reads the same but Discord no longer parses it as a mention.
fn defuse_mass_pings(text: &str) -> String {
    text.replace("@everyone", "@\u{200b}everyone")
        .replace("@here", "@\u{200b}here")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MentionKind {
    Role,
    User,
}

struct Mention {
    start: usize,
    end: usize,
    kind: MentionKind,
}

/// Scans for Discord mention syntax: `<@&digits>` (role) or `<@digits>` /
/// `<@!digits>` (user). Every delimiter involved is ASCII, so byte offsets here
/// always land on `char` boundaries.
fn find_mentions(text: &str) -> Vec<Mention> {
    let bytes = text.as_bytes();
    let mut mentions = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' && bytes.get(i + 1) == Some(&b'@') {
            let mut cursor = i + 2;
            let kind = if bytes.get(cursor) == Some(&b'&') {
                cursor += 1;
                MentionKind::Role
            } else {
                if bytes.get(cursor) == Some(&b'!') {
                    cursor += 1;
                }
                MentionKind::User
            };

            let digits_start = cursor;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }

            if cursor > digits_start && bytes.get(cursor) == Some(&b'>') {
                mentions.push(Mention {
                    start: i,
                    end: cursor + 1,
                    kind,
                });
                i = cursor + 1;
                continue;
            }
        }
        i += 1;
    }

    mentions
}

/// Strips role mentions entirely and collapses user mentions into a single note
/// once there are more than [`MASS_MENTION_THRESHOLD`] of them.
fn filter_mentions(text: &str) -> String {
    let mentions = find_mentions(text);
    let user_mention_count = mentions
        .iter()
        .filter(|mention| mention.kind == MentionKind::User)
        .count();
    let collapse_users = user_mention_count > MASS_MENTION_THRESHOLD;

    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    for mention in &mentions {
        result.push_str(&text[cursor..mention.start]);
        match mention.kind {
            MentionKind::Role => {}
            MentionKind::User if collapse_users => {}
            MentionKind::User => result.push_str(&text[mention.start..mention.end]),
        }
        cursor = mention.end;
    }
    result.push_str(&text[cursor..]);

    if collapse_users {
        result.push_str(&format!(" ({user_mention_count} mentions collapsed)"));
    }

    result
}

/// Truncates `text` to at most `max_length` characters, replacing the last kept
/// character with an ellipsis when truncation happens.
fn truncate_with_ellipsis(text: &str, max_length: usize) -> String {
    if text.chars().count() <= max_length {
        return text.to_string();
    }
    if max_length == 0 {
        return String::new();
    }

    let keep = max_length - 1;
    let mut truncated: String = text.chars().take(keep).collect();
    truncated.push('…');
    truncated
}

/// Runs `decancer` over `text` with `retain_capitalization` enabled, so plain
/// ASCII and accented Latin text keeps its case instead of being forced to
/// lowercase. Falls back to the untouched input if the text is too malformed
/// for decancer's bidirectional text handling to process.
fn decancer_output(text: &str) -> String {
    let options = Options::default().retain_capitalization();
    match decancer::cure(text, options) {
        Ok(cured) => String::from(cured),
        Err(_) => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_length: usize) -> OutputLimits {
        OutputLimits::new(max_length)
    }

    #[test]
    fn test_plain_text_passes_through_unchanged() {
        assert_eq!(
            filter_output("just a normal response", limits(2000)),
            "just a normal response"
        );
    }

    #[test]
    fn test_everyone_ping_is_defused() {
        let filtered = filter_output("watch out @everyone!", limits(2000));
        assert!(!filtered.contains("@everyone"));
        assert!(filtered.contains("everyone"));
    }

    #[test]
    fn test_here_ping_is_defused() {
        let filtered = filter_output("@here, look at this", limits(2000));
        assert!(!filtered.contains("@here"));
        assert!(filtered.contains("here"));
    }

    #[test]
    fn test_role_mention_is_stripped() {
        let filtered = filter_output("ping <@&123456789012345678> for help", limits(2000));
        assert!(!filtered.contains("<@&"));
    }

    #[test]
    fn test_a_few_user_mentions_are_left_alone() {
        let text = "as <@1> and <@2> said";
        assert_eq!(filter_output(text, limits(2000)), text);
    }

    #[test]
    fn test_a_flood_of_user_mentions_is_collapsed() {
        let text = "<@1> <@2> <@3> <@4> <@5>";
        let filtered = filter_output(text, limits(2000));
        assert!(!filtered.contains("<@1>"));
        assert!(!filtered.contains("<@5>"));
        assert!(filtered.contains("5 mentions collapsed"));
    }

    #[test]
    fn test_nickname_style_user_mention_is_recognized() {
        let text = "<@!42> said hi";
        assert_eq!(filter_output(text, limits(2000)), text);
    }

    #[test]
    fn test_text_within_the_limit_is_not_truncated() {
        let text = "short";
        assert_eq!(filter_output(text, limits(5)), "short");
    }

    #[test]
    fn test_text_over_the_limit_is_truncated_with_an_ellipsis() {
        let filtered = filter_output("hello world", limits(6));
        assert_eq!(filtered.chars().count(), 6);
        assert!(filtered.ends_with('…'));
        assert!(filtered.starts_with("hello"));
    }

    #[test]
    fn test_truncation_counts_characters_not_bytes() {
        // each "é" is two bytes but one character, so byte-based truncation would
        // either panic on a non-boundary split or keep too little visible text
        let filtered = filter_output("ééééé", limits(3));
        assert_eq!(filtered.chars().count(), 3);
    }

    #[test]
    fn test_confusable_characters_are_normalized() {
        let filtered = filter_output("vＥⓡ𝔂 𝔽𝕌Ňℕｙ", limits(2000));
        assert_eq!(filtered, "very funny");
    }

    #[test]
    fn test_capitalization_is_preserved_through_decancer() {
        let filtered = filter_output("Hello World", limits(2000));
        assert_eq!(filtered, "Hello World");
    }

    #[test]
    fn test_zero_max_length_yields_empty_output() {
        assert_eq!(filter_output("anything", limits(0)), "");
    }

    #[test]
    fn test_length_limit_holds_even_when_decancer_expands_a_character() {
        // decancer maps a literal "…" to three ASCII periods, expanding the text by
        // two characters. If truncation ran before decancer instead of after, this
        // input would slip past the limit.
        let filtered = filter_output("ok…", limits(3));
        assert!(
            filtered.chars().count() <= 3,
            "output exceeded the limit: {filtered:?}"
        );
    }
}
