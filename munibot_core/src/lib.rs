// munibot_core: platform-agnostic core library for munibot.

pub mod config;
pub mod db;
pub mod error;
pub mod greeting;
pub mod magical;
pub mod passing;
pub mod permission;

pub use config::{Config, DiscordConfig, TwitchConfig};
pub use db::{DbPool, establish_pool, run_pending_migrations};
pub use error::MuniBotError;
pub use passing::Passing;
pub use permission::Permission;
