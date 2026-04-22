use crate::twitch::handler::TwitchMessageHandler;

pub mod affection;
pub mod autoban;
pub mod bonk;
pub mod bot_affection;
pub mod content_warning;
pub mod dice;
pub mod economy;
pub mod eight_ball;
pub mod greeting;
pub mod lift;
pub mod lurk;
pub mod magical;
pub mod quotes;
pub mod shoutout;
pub mod socials;
pub mod temperature;
pub mod ventriloquize;

pub type TwitchHandlerCollection = Vec<Box<dyn TwitchMessageHandler>>;

// Re-export discord collection types from munibot_discord.
pub use munibot_discord::{
    commands::DiscordCommandProviderCollection, state::DiscordMessageHandlerCollection,
};
