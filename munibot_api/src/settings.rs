//! Wire types and errors for the guild/user settings pages.

mod channel;
mod error;
mod logging;

pub use channel::ChannelSummary;
pub use error::{SettingsError, SettingsResult};
pub use logging::GuildLoggingSettings;
