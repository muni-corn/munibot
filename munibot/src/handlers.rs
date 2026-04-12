use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    discord::{commands::DiscordCommandProvider, handler::DiscordEventHandler},
    twitch::handler::TwitchMessageHandler,
};

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
pub mod logging;
pub mod lurk;
pub mod magical;
pub mod quotes;
pub mod shoutout;
pub mod socials;
pub mod temperature;
pub mod ventriloquize;

pub type TwitchHandlerCollection = Vec<Box<dyn TwitchMessageHandler>>;
pub type DiscordMessageHandlerCollection = Vec<Arc<Mutex<dyn DiscordEventHandler>>>;
pub type DiscordCommandProviderCollection = Vec<Box<dyn DiscordCommandProvider>>;
