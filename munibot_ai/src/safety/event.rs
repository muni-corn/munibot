use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::limits::Scope;

/// Which safety system produced an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyEventType {
    RateLimit,
    SpendCap,
    Moderation,
    Crisis,
}

impl SafetyEventType {
    /// The stable string this event type is stored as, mirroring
    /// [`crate::audit::ToolCallStatus::as_key`].
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::SpendCap => "spend_cap",
            Self::Moderation => "moderation",
            Self::Crisis => "crisis",
        }
    }
}

/// One safety system tripping: a rate-limit refusal, a spend cap refusal,
/// a moderation block, or a crisis classifier trigger.
///
/// Deliberately excludes raw content - see [`Self::content_hash`]'s own
/// doc comment - so `ai_safety_events` can be reviewed to tune every
/// safety system without becoming a second place a user's own words end
/// up stored.
#[derive(Clone, Debug, PartialEq)]
pub struct SafetyEvent {
    pub event_type: SafetyEventType,
    pub scope: Scope,
    /// A short, stable, human-readable description of why - an error's
    /// own `Display`, a moderation category list, a crisis severity name.
    /// Never raw content.
    pub reason: String,
    /// A SHA-256 hex digest of the content that tripped this event, when
    /// there was meaningfully "content" to hash at all - a rate limit or
    /// spend cap trip has none, so this stays `None` for both of those.
    /// Lets an operator confirm two events came from the same repeated
    /// message without this table ever holding that message itself.
    pub content_hash: Option<String>,
}

impl SafetyEvent {
    pub fn new(event_type: SafetyEventType, scope: Scope, reason: impl Into<String>) -> Self {
        Self {
            event_type,
            scope,
            reason: reason.into(),
            content_hash: None,
        }
    }

    /// Attaches a content hash, for an event kind that actually has
    /// content worth correlating (moderation, crisis).
    pub fn with_content(mut self, content: &str) -> Self {
        self.content_hash = Some(hash_content(content));
        self
    }
}

/// Renders `text`'s SHA-256 digest as lowercase hex - the one-way
/// transform that lets `ai_safety_events` correlate repeated content
/// without ever storing it.
pub fn hash_content(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Records one safety event.
///
/// Must never propagate a failure to the caller - the same reasoning
/// [`crate::audit::ToolAuditor`] documents: auditing failing to write must
/// never affect whether the safety check itself did its job.
#[async_trait]
pub trait SafetyEventAuditor: Send + Sync {
    async fn record(&self, event: SafetyEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_keys_are_stable_short_strings() {
        assert_eq!(SafetyEventType::RateLimit.as_key(), "rate_limit");
        assert_eq!(SafetyEventType::SpendCap.as_key(), "spend_cap");
        assert_eq!(SafetyEventType::Moderation.as_key(), "moderation");
        assert_eq!(SafetyEventType::Crisis.as_key(), "crisis");
    }

    #[test]
    fn test_hash_content_is_a_deterministic_sha256_hex_digest() {
        // known sha256("hello") - a fixed vector, not just "same input same output"
        assert_eq!(
            hash_content("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_hash_content_differs_for_different_input() {
        assert_ne!(hash_content("hello"), hash_content("goodbye"));
    }

    #[test]
    fn test_new_event_has_no_content_hash_by_default() {
        let event = SafetyEvent::new(
            SafetyEventType::RateLimit,
            Scope::Global,
            "too many requests",
        );
        assert!(event.content_hash.is_none());
    }

    #[test]
    fn test_with_content_attaches_a_hash() {
        let event = SafetyEvent::new(SafetyEventType::Moderation, Scope::User(1), "flagged")
            .with_content("some flagged text");
        assert_eq!(event.content_hash, Some(hash_content("some flagged text")));
    }
}
