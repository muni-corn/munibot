//! Wire types and errors for the guild/user settings pages.

mod ai;
mod channel;
mod error;
mod logging;

pub use ai::{CHANNEL_MODE_ALL, CHANNEL_MODE_ALLOWLIST, GuildAiSettings};
pub use channel::ChannelSummary;
pub use error::{SettingsError, SettingsResult};
pub use logging::GuildLoggingSettings;
