use crate::twitch::handler::TwitchMessageHandler;

// Twitch-only handlers (still in binary until munibot_twitch is created)
pub mod affection;
pub mod autoban;
pub mod bonk;
pub mod content_warning;
pub mod greeting;
pub mod lift;
pub mod lurk;
pub mod magical;
pub mod quotes;
pub mod shoutout;
pub mod socials;

// Discord-only handlers (now in munibot_discord)
pub use munibot_discord::handlers::{
    bot_affection, dice, economy, eight_ball, temperature, ventriloquize,
};

pub type TwitchHandlerCollection = Vec<Box<dyn TwitchMessageHandler>>;

// Re-export discord collection types from munibot_discord.
pub use munibot_discord::{
    commands::DiscordCommandProviderCollection, state::DiscordMessageHandlerCollection,
};
