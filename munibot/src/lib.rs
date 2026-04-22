#![feature(never_type)]

pub use munibot_core::error::MuniBotError as CoreError;
use poise::serenity_prelude as serenity;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use twitch_irc::login::UserAccessToken;

use crate::discord::commands::DiscordCommandError;

pub mod config;
pub mod db;
pub mod discord;
pub mod handlers;
pub mod passing;
pub mod twitch;

#[derive(Error, Debug)]
pub enum MuniBotError {
    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("token send failed :< {0}")]
    SendError(#[from] SendError<UserAccessToken>),

    #[error("discord `{0}` command failed :< `{1}`")]
    DiscordCommand(String, String),

    #[error("error in discord framework :< {0}")]
    SerenityError(#[from] Box<serenity::Error>),
}

impl From<DiscordCommandError> for MuniBotError {
    fn from(e: DiscordCommandError) -> Self {
        Self::DiscordCommand(e.command_identifier.to_string(), format!("{e}"))
    }
}

impl From<anyhow::Error> for MuniBotError {
    fn from(value: anyhow::Error) -> Self {
        Self::Core(CoreError::Other(value.to_string()))
    }
}

impl From<serenity::Error> for MuniBotError {
    fn from(e: serenity::Error) -> Self {
        Self::SerenityError(Box::new(e))
    }
}

// Delegate From impls for core error subtypes so the ? operator works in binary
// code that still returns MuniBotError directly.

impl From<serde_json::Error> for MuniBotError {
    fn from(e: serde_json::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<reqwest::Error> for MuniBotError {
    fn from(e: reqwest::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<diesel::result::Error> for MuniBotError {
    fn from(e: diesel::result::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<humantime::DurationError> for MuniBotError {
    fn from(e: humantime::DurationError) -> Self {
        Self::Core(e.into())
    }
}
