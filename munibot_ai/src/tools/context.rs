use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{tools::RiskTier, types::AiError};

/// A stable identifier for one stored conversation.
///
/// Assigned by the memory module's session store; tools only ever borrow it to
/// scope their own reads and writes. Defined here rather than in a `memory`
/// module of its own, since `ToolCtx` is the first thing that needs it - the
/// eventual session store reuses this same type rather than inventing a second
/// one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ConversationId(pub u64);

/// Which chat platform a tool invocation originated from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Discord,
    Twitch,
    Web,
}

impl Platform {
    /// The stable string this platform is stored as in the database.
    ///
    /// Distinct from [`Display`](std::fmt::Display), which is prose for a
    /// system prompt and renders `Web` as "the web" - fine to read, useless as
    /// a key. Same reasoning as [`crate::types::Role::as_key`]: persisted
    /// strings should not move when a display string is reworded.
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Twitch => "twitch",
            Self::Web => "web",
        }
    }

    /// Parses a platform back from its stored string.
    pub fn from_key(text: &str) -> Option<Self> {
        match text {
            "discord" => Some(Self::Discord),
            "twitch" => Some(Self::Twitch),
            "web" => Some(Self::Web),
            _ => None,
        }
    }
}

impl std::fmt::Display for Platform {
    /// A human-readable form, for rendering into a persona's `{{platform}}`
    /// system prompt variable - "you're talking with muni on Discord" reads
    /// naturally, "you're talking with muni on Web" does not.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Discord => "Discord",
            Self::Twitch => "Twitch",
            Self::Web => "the web",
        };
        write!(f, "{name}")
    }
}

/// Everything a tool needs to know about who is invoking it and where.
///
/// A tool's authority derives entirely from this context, never from what the
/// model asks for. Every tool above [`RiskTier::Safe`] must call
/// [`Self::require_tier`] as its first action.
#[derive(Clone, Debug)]
pub struct ToolCtx {
    /// The invoking human's internal `users.id`, never a raw platform
    /// snowflake, so memory and usage records survive a user linking a
    /// second platform account. See `docs/notes/gui-configuration-research.
    /// md` for the identity trap this avoids.
    pub user_id: u64,
    pub platform: Platform,
    /// The tier this invocation is authorized for, set once by the adapter that
    /// received the message from the invoker's actual permissions - never
    /// from persona configuration alone, and never widened by a tool
    /// itself.
    pub granted_tier: RiskTier,
    /// The Discord guild this invocation happened in, when there is one.
    pub guild_id: Option<u64>,
    pub conversation_id: ConversationId,
    pub cancellation: CancellationToken,
}

impl ToolCtx {
    /// Fails unless this context is authorized for at least `tier`.
    ///
    /// Every tool above [`RiskTier::Safe`] should call this as its first line,
    /// so a persona misconfigured into a tier the invoker lacks is refused
    /// at the point of use rather than trusted to have already been
    /// filtered out of the schema list.
    pub fn require_tier(&self, tier: RiskTier) -> Result<(), AiError> {
        if self.granted_tier >= tier {
            Ok(())
        } else {
            Err(AiError::Tool(format!(
                "this needs {tier:?} authorization, but the invoker only has {:?} :<",
                self.granted_tier
            )))
        }
    }

    /// Returns `true` once this invocation's cancellation token has been
    /// triggered.
    ///
    /// A long-running tool (a sandboxed build, a slow search) should check this
    /// periodically and stop promptly rather than only at its next await
    /// point.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier,
            guild_id: Some(42),
            conversation_id: ConversationId(7),
            cancellation: CancellationToken::new(),
        }
    }

    #[test]
    fn test_require_tier_passes_at_exactly_the_granted_tier() {
        assert!(
            ctx(RiskTier::NetworkRead)
                .require_tier(RiskTier::NetworkRead)
                .is_ok()
        );
    }

    #[test]
    fn test_require_tier_passes_below_the_granted_tier() {
        assert!(
            ctx(RiskTier::Sandbox).require_tier(RiskTier::Safe).is_ok(),
            "a higher granted tier should satisfy a lower requirement"
        );
    }

    #[test]
    fn test_require_tier_fails_above_the_granted_tier() {
        let result = ctx(RiskTier::Safe).require_tier(RiskTier::Privileged);
        assert!(
            result.is_err(),
            "a Safe-only context must not be able to invoke a Privileged action"
        );
    }

    #[test]
    fn test_require_tier_error_names_both_tiers() {
        let error = ctx(RiskTier::Safe)
            .require_tier(RiskTier::Sandbox)
            .expect_err("should fail");
        let message = error.to_string();
        assert!(
            message.contains("Sandbox") && message.contains("Safe"),
            "the error should say both what was needed and what was granted: {message:?}"
        );
    }

    #[test]
    fn test_is_cancelled_reflects_the_token() {
        let context = ctx(RiskTier::Safe);
        assert!(
            !context.is_cancelled(),
            "a fresh token should not be cancelled"
        );

        context.cancellation.cancel();
        assert!(
            context.is_cancelled(),
            "cancelling the token should be visible through the context"
        );
    }

    #[test]
    fn test_conversation_id_serializes_as_a_bare_integer() {
        let encoded = serde_json::to_value(ConversationId(42)).expect("should serialize");
        assert_eq!(encoded, serde_json::json!(42));
    }

    #[test]
    fn test_platform_serializes_as_snake_case() {
        let encoded = serde_json::to_string(&Platform::Discord).expect("should serialize");
        assert_eq!(encoded, "\"discord\"");
    }

    #[test]
    fn test_platform_displays_a_human_readable_name() {
        assert_eq!(Platform::Discord.to_string(), "Discord");
        assert_eq!(Platform::Twitch.to_string(), "Twitch");
        assert_eq!(Platform::Web.to_string(), "the web");
    }

    #[test]
    fn test_platform_key_round_trips_for_every_variant() {
        for platform in [Platform::Discord, Platform::Twitch, Platform::Web] {
            assert_eq!(Platform::from_key(platform.as_key()), Some(platform));
        }
    }

    #[test]
    fn test_platform_key_is_not_the_display_string() {
        // "the web" reads well in a prompt and is useless as a database key
        assert_eq!(Platform::Web.as_key(), "web");
        assert_eq!(Platform::Web.to_string(), "the web");
    }
}
