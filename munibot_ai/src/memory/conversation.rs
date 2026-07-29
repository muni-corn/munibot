use chrono::{DateTime, Utc};

use crate::tools::{ConversationId, Platform};

/// Which conversation a message belongs to, before it has been assigned a
/// [`ConversationId`].
///
/// The scope is stable across restarts (a channel, a thread, a direct message),
/// while the id is only assigned once a row for that scope exists. A platform
/// adapter always starts from a scope,
/// and [`crate::memory::SessionStore::load_or_create`] is what turns one into
/// an id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConversationScope {
    pub platform: Platform,
    /// A channel, thread, or direct-message identifier, unique within its
    /// platform.
    pub scope_key: String,
}

impl ConversationScope {
    pub fn new(platform: Platform, scope_key: impl Into<String>) -> Self {
        Self {
            platform,
            scope_key: scope_key.into(),
        }
    }
}

/// One stored conversation.
///
/// `persona_id` is a plain string rather than a `PersonaId` newtype: the
/// persona module lands later in this milestone, and this type has no reason to
/// wait on it - a session store only ever stores and returns the identifier,
/// never validates it against a persona registry.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: ConversationId,
    pub scope: ConversationScope,
    pub persona_id: String,
    /// A condensed summary of older messages, once the conversation has been
    /// compacted. `None` until compaction lands in milestone 2.
    pub summary: Option<String>,
    pub last_active_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_equality_depends_on_both_platform_and_key() {
        let a = ConversationScope::new(Platform::Discord, "channel-1");
        let b = ConversationScope::new(Platform::Discord, "channel-1");
        let different_platform = ConversationScope::new(Platform::Twitch, "channel-1");
        let different_key = ConversationScope::new(Platform::Discord, "channel-2");

        assert_eq!(a, b, "identical platform and key should be equal");
        assert_ne!(
            a, different_platform,
            "the same key on a different platform is a different scope"
        );
        assert_ne!(
            a, different_key,
            "a different key on the same platform is a different scope"
        );
    }

    #[test]
    fn test_scope_can_be_used_as_a_hashmap_key() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            ConversationScope::new(Platform::Discord, "channel-1"),
            "first",
        );
        map.insert(
            ConversationScope::new(Platform::Twitch, "channel-1"),
            "second",
        );

        assert_eq!(
            map.len(),
            2,
            "the two platforms should not collide despite the same key"
        );
        assert_eq!(
            map[&ConversationScope::new(Platform::Discord, "channel-1")],
            "first"
        );
    }

    #[test]
    fn test_conversation_carries_no_summary_until_compacted() {
        let conversation = Conversation {
            id: ConversationId(1),
            scope: ConversationScope::new(Platform::Discord, "channel-1"),
            persona_id: "companion".to_string(),
            summary: None,
            last_active_at: Utc::now(),
        };
        assert_eq!(conversation.summary, None);
    }
}
