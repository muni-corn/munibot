//! Wire types and errors for the guild/user settings pages.

mod channel;
mod error;

pub use channel::ChannelSummary;
pub use error::{SettingsError, SettingsResult};
