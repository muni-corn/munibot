#![feature(decl_macro)]
#![feature(never_type)]
#![feature(duration_constructors_lite)]

use poise::serenity_prelude as serenity;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use twitch_irc::login::UserAccessToken;

use crate::discord::commands::DiscordCommandError;

pub mod config;
pub mod db;
pub mod discord;
pub mod handlers;
pub mod twitch;

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

    #[error("couldn't parse duration :< {0}")]
    DurationParseError(#[from] humantime::DurationError),

    #[error("something went wrong :< {0}")]
    Other(String),
}

impl From<DiscordCommandError> for MuniBotError {
    fn from(e: DiscordCommandError) -> Self {
        Self::DiscordCommand(e.command_identifier.to_string(), format!("{e}"))
    }
}

impl From<anyhow::Error> for MuniBotError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value.to_string())
    }
}
