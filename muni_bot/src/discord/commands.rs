use muni_bot_core::MuniBotError;

use super::DiscordState;

pub trait DiscordCommandProvider: Send {
    fn commands(&self) -> Vec<poise::Command<DiscordState, MuniBotError>>;
}
