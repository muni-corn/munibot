use serde::{Deserialize, Serialize};

use crate::types::content::{ContentBlock, Role};

/// One turn in a conversation.
///
/// A message holds several blocks because a single assistant turn can mix
/// reasoning and text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// Builds a message from a role and its blocks.
    pub fn new(role: Role, content: Vec<ContentBlock>) -> Self {
        Self { role, content }
    }

    /// Builds a message from the human talking to munibot.
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![ContentBlock::text(text)])
    }

    /// Builds a message from munibot.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, vec![ContentBlock::text(text)])
    }

    /// Concatenates every text block, ignoring reasoning.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join("")
    }
}

/// An ordered conversation.
///
/// Serialized transparently as a JSON array, so provider payloads see a plain
/// list of messages.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
pub struct History(Vec<Message>);

impl History {
    /// Builds an empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a message.
    pub fn push(&mut self, message: Message) {
        self.0.push(message);
    }

    /// Iterates messages oldest first.
    pub fn iter(&self) -> std::slice::Iter<'_, Message> {
        self.0.iter()
    }

    /// Borrows the messages as a slice.
    pub fn messages(&self) -> &[Message] {
        &self.0
    }

    /// Consumes the history, yielding its messages.
    pub fn into_messages(self) -> Vec<Message> {
        self.0
    }

    /// Returns the most recent message, or `None` when empty.
    pub fn last(&self) -> Option<&Message> {
        self.0.last()
    }

    /// Returns the number of messages.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` when there are no messages.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<Message>> for History {
    fn from(messages: Vec<Message>) -> Self {
        Self(messages)
    }
}

impl FromIterator<Message> for History {
    fn from_iter<T: IntoIterator<Item = Message>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for History {
    type IntoIter = std::vec::IntoIter<Message>;
    type Item = Message;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_roundtrips() {
        let message = Message::user("hello");
        let encoded = serde_json::to_string(&message).expect("should serialize");
        let decoded: Message = serde_json::from_str(&encoded).expect("should deserialize");
        assert_eq!(decoded, message, "message should survive a roundtrip");
    }

    #[test]
    fn test_text_concatenates_only_text_blocks() {
        let message = Message::new(Role::Assistant, vec![
            ContentBlock::thinking("hmm"),
            ContentBlock::text("the answer "),
            ContentBlock::text("is 4"),
        ]);
        assert_eq!(
            message.text(),
            "the answer is 4",
            "text should skip reasoning"
        );
    }

    #[test]
    fn test_history_serializes_as_a_bare_array() {
        let history = History::from(vec![Message::user("hi")]);
        let encoded = serde_json::to_value(&history).expect("should serialize");
        assert!(
            encoded.is_array(),
            "history should be a transparent array, got {encoded}"
        );
    }

    #[test]
    fn test_history_push_and_len() {
        let mut history = History::new();
        assert!(history.is_empty(), "a new history should be empty");

        history.push(Message::user("one"));
        history.push(Message::assistant("two"));

        assert_eq!(history.len(), 2, "two messages should have been recorded");
        assert_eq!(
            history.last().map(Message::text),
            Some("two".to_string()),
            "last should return the most recent message"
        );
    }

    #[test]
    fn test_token_estimate_counts_tool_traffic() {
        let text_only = History::from(vec![Message::user("hello")]);
        let with_call = History::from(vec![Message::new(Role::Assistant, vec![
            ContentBlock::tool_use(
                "c",
                "web_search",
                json!({"query": "a long search query here"}),
            ),
        ])]);

        assert!(
            with_call.token_estimate(rough_token_estimate)
                > text_only.token_estimate(rough_token_estimate),
            "tool arguments consume context and must be counted"
        );
    }

    #[test]
    fn test_token_estimate_ignores_images() {
        let history = History::from(vec![Message::new(Role::User, vec![ContentBlock::Image {
            image: crate::types::content::Image {
                media_type: "image/png".to_string(),
                source: crate::types::content::ImageSource::Base64 {
                    // a long base64 payload must not be mistaken for text tokens
                    data: "A".repeat(10_000),
                },
            },
        }])]);
        assert_eq!(
            history.token_estimate(rough_token_estimate),
            0,
            "image bytes are not text tokens"
        );
    }

    #[test]
    fn test_rough_token_estimate_rounds_up() {
        assert_eq!(rough_token_estimate(""), 0, "empty text is zero tokens");
        assert_eq!(
            rough_token_estimate("a"),
            1,
            "a single character still costs a token"
        );
        assert_eq!(
            rough_token_estimate("abcd"),
            1,
            "four characters is one token"
        );
        assert_eq!(
            rough_token_estimate("abcde"),
            2,
            "five characters rounds up to two"
        );
    }

    #[test]
    fn test_history_collects_from_iterator() {
        let history: History = vec![Message::user("a"), Message::assistant("b")]
            .into_iter()
            .collect();
        assert_eq!(history.len(), 2, "collect should preserve both messages");
    }
}
