use async_trait::async_trait;
use munibot_core::{config::Config, magical::get_magic_message};
use twitch_irc::message::ServerMessage;

use crate::{
    agent::TwitchAgent,
    bot::MuniBotTwitchIRCClient,
    handler::{TwitchHandlerError, TwitchMessageHandler},
};

/// Twitch magical handler. Uses munibot_core::magical for the calculation,
/// implementing only the Twitch side.
pub struct MagicalHandler;

#[async_trait]
impl TwitchMessageHandler for MagicalHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = match message {
            ServerMessage::Privmsg(msg) if msg.message_text.starts_with("!magical") => {
                self.send_twitch_message(
                    client,
                    &msg.channel_login,
                    &get_magic_message(&msg.sender.id, &msg.sender.name),
                )
                .await?;
                true
            }
            _ => false,
        };

        Ok(handled)
    }
}
