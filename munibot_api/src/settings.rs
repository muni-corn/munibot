//! Wire types and errors for the guild/user settings pages.

mod channel;
mod error;
mod logging;
mod logging_page;

pub use channel::ChannelSummary;
pub use error::{SettingsError, SettingsResult};
pub use logging::GuildLoggingSettings;
pub use logging_page::GuildLoggingPage;
