use std::fmt::Display;

use async_trait::async_trait;
use twitch_irc::message::ServerMessage;

use super::agent::{TwitchAgent, TwitchAgentError};
use crate::{
    config::Config,
    twitch::bot::{MuniBotTwitchIRCClient, MuniBotTwitchIRCError},
};

#[async_trait]
pub trait TwitchMessageHandler: Send {
    async fn send_twitch_message(
        &mut self,
        irc_client: &MuniBotTwitchIRCClient,
        channel_login: &str,
        message: &str,
    ) -> Result<(), TwitchHandlerError> {
        irc_client
            .say(channel_login.to_string(), message.to_string())
            .await
            .map_err(TwitchHandlerError::SendMessage)
    }

    /// Handle a new message from chat. Returns `true` if something was done to
    /// handle the message, or `false` if the message was ignored (or if the
    /// message is allowed to also be handled by other handlers).
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        irc_client: &MuniBotTwitchIRCClient,
        agent: &TwitchAgent,
        config: &Config,
    ) -> Result<bool, TwitchHandlerError>;
}

#[derive(Debug)]
pub enum TwitchHandlerError {
    SendMessage(MuniBotTwitchIRCError),
    TwitchIRCError(MuniBotTwitchIRCError),
    DbError(surrealdb::Error),
    AgentError(TwitchAgentError),
    Other(String),
}

impl From<MuniBotTwitchIRCError> for TwitchHandlerError {
    fn from(e: MuniBotTwitchIRCError) -> Self {
        Self::TwitchIRCError(e)
    }
}

impl From<surrealdb::Error> for TwitchHandlerError {
    fn from(e: surrealdb::Error) -> Self {
        Self::DbError(e)
    }
}

impl From<TwitchAgentError> for TwitchHandlerError {
    fn from(e: TwitchAgentError) -> Self {
        Self::AgentError(e)
    }
}

impl Display for TwitchHandlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TwitchHandlerError::SendMessage(e) => write!(f, "message send failure! {e}"),
            TwitchHandlerError::TwitchIRCError(e) => write!(f, "irc error :< {e}"),
            TwitchHandlerError::DbError(e) => write!(f, "database error :( {e}"),
            TwitchHandlerError::AgentError(e) => write!(f, "twitch agent error: {e}"),
            TwitchHandlerError::Other(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TwitchHandlerError {}
