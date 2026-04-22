// munibot_core: platform-agnostic core library for munibot.

pub mod config;
pub mod error;
pub mod passing;

pub use config::{Config, DiscordConfig, TwitchConfig};
pub use error::MuniBotError;
pub use passing::Passing;
