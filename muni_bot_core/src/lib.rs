use std::fmt::{self, Display};

use poise::serenity_prelude as serenity;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use twitch_irc::login::UserAccessToken;

pub mod config;
pub mod db;

#[derive(Error, Debug)]
pub enum MuniBotError {
    #[error("parsing failure :< {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("request failed :< {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("token send failed :< {0}")]
    SendError(#[from] SendError<UserAccessToken>),

    #[error("discord `{0}` command failed :< `{1}`")]
    DiscordCommand(String, String),

    #[error("missing token :<")]
    MissingToken,

    #[error("error with database :< {0}")]
    DbError(#[from] surrealdb::Error),

    #[error("error in discord framework :< {0}")]
    SerenityError(#[from] serenity::Error),

    #[error("error loading config :< {0}, {1}")]
    LoadConfig(String, anyhow::Error),

    #[error("something different went wrong :< {0}")]
    Other(String),
}

impl From<DiscordCommandError> for MuniBotError {
    fn from(e: DiscordCommandError) -> Self {
        Self::DiscordCommand(e.command_identifier.to_string(), format!("{e}"))
    }
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
impl std::error::Error for DiscordCommandError {}
