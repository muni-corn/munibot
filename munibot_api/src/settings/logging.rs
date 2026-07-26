use serde::{Deserialize, Serialize};

/// A guild's logging settings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct GuildLoggingSettings {
    /// The channel server events are logged to, as a snowflake string
    /// (matching `GuildSummary.id`/`ChannelSummary.id`). `None` means
    /// logging is turned off.
    pub channel_id: Option<String>,
}
