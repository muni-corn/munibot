use std::fmt::Display;

use async_trait::async_trait;
use poise::serenity_prelude::{self as serenity, FullEvent};
use thiserror::Error;

use super::DiscordFrameworkContext;

#[async_trait]
pub trait DiscordEventHandler: Sync + Send {
    fn name(&self) -> &'static str;
    async fn handle_discord_event(
        &mut self,
        context: &serenity::Context,
        framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError>;
}

#[derive(Error, Debug)]
#[error("error in discord handler {handler_name}: {message}")]
pub struct DiscordHandlerError {
    pub handler_name: &'static str,
    pub message: String,
}

impl DiscordHandlerError {
    pub fn from_display(handler_name: &'static str, error: impl Display) -> Self {
        Self {
            handler_name,
            message: format!("{error}"),
        }
    }
}
