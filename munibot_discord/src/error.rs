use munibot_core::error::MuniBotError as CoreError;
use poise::serenity_prelude as serenity;
use thiserror::Error;

use crate::commands::DiscordCommandError;

/// Discord-specific error type. Wraps the core error and adds Discord and
/// serenity-specific variants.
#[derive(Error, Debug)]
pub enum MunibotDiscordError {
    #[error(transparent)]
    Core(#[from] CoreError),

    #[error("discord `{0}` command failed :< `{1}`")]
    Command(String, String),

    #[error("error in discord framework :< {0}")]
    Serenity(#[from] Box<serenity::Error>),
}

impl From<DiscordCommandError> for MunibotDiscordError {
    fn from(e: DiscordCommandError) -> Self {
        Self::Command(e.command_identifier.to_string(), format!("{e}"))
    }
}

impl From<anyhow::Error> for MunibotDiscordError {
    fn from(value: anyhow::Error) -> Self {
        Self::Core(CoreError::Other(value.to_string()))
    }
}

impl From<serenity::Error> for MunibotDiscordError {
    fn from(e: serenity::Error) -> Self {
        Self::Serenity(Box::new(e))
    }
}

// Delegate From impls for core error subtypes so ? works in discord code.

impl From<serde_json::Error> for MunibotDiscordError {
    fn from(e: serde_json::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<diesel::result::Error> for MunibotDiscordError {
    fn from(e: diesel::result::Error) -> Self {
        Self::Core(e.into())
    }
}

impl From<humantime::DurationError> for MunibotDiscordError {
    fn from(e: humantime::DurationError) -> Self {
        Self::Core(e.into())
    }
}
