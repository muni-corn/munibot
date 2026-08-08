use serde::{Deserialize, Serialize};

use crate::settings::{ChannelSummary, GuildLoggingSettings};

/// Everything the logging settings page needs to render: the guild's
/// postable channels (for the picker) and its current logging settings.
/// Bundled into one type so the page can be loaded with a single server
/// function call and a single guild-admin check, instead of one of each per
/// piece of data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GuildLoggingPage {
    pub channels: Vec<ChannelSummary>,
    pub settings: GuildLoggingSettings,
}
