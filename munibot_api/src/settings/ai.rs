use serde::{Deserialize, Serialize};

/// The two shapes `channel_mode` may take - a plain string over the wire
/// (like the database column it mirrors, `guild_configs.ai_channel_mode`),
/// but named constants here so a client and `set_guild_ai_settings` never
/// have to agree on the literal by coincidence.
pub const CHANNEL_MODE_ALL: &str = "all";
pub const CHANNEL_MODE_ALLOWLIST: &str = "allowlist";

/// A guild's ai settings.
///
/// Deliberately no `#[derive(Default)]`: `channel_mode` has no sensible
/// zero value (an empty string isn't [`CHANNEL_MODE_ALL`] or
/// [`CHANNEL_MODE_ALLOWLIST`]) the way `GuildLoggingSettings`'s single
/// `Option` field does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GuildAiSettings {
    pub enabled: bool,
    /// A persona id, overriding the service-wide default for this guild.
    /// `None` means "use whatever `Ai::default_persona_id` resolves to".
    pub default_persona: Option<String>,
    /// [`CHANNEL_MODE_ALL`] or [`CHANNEL_MODE_ALLOWLIST`].
    pub channel_mode: String,
    /// Channel snowflakes (matching `GuildSummary.id`/`ChannelSummary.id`),
    /// consulted only when `channel_mode` is [`CHANNEL_MODE_ALLOWLIST`] -
    /// still round-tripped either way, so switching back to allowlist mode
    /// later doesn't lose a previously-saved list.
    pub channel_allowlist: Vec<String>,
}
