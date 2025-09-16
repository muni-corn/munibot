use std::{
    error::Error,
    fmt::{self, Display},
};

use super::DiscordState;
use crate::MunibotError;

pub trait DiscordCommandProvider: Send {
    fn commands(&self) -> Vec<poise::Command<DiscordState, MunibotError>>;
}

#[derive(Debug)]
pub struct DiscordCommandError {
    pub message: String,
    pub command_identifier: String,
}
impl Display for DiscordCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the `{}` discord command encountered an error: {}",
            self.command_identifier, self.message
        )
    }
}
impl Error for DiscordCommandError {}
