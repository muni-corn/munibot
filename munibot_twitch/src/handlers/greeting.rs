use async_trait::async_trait;
use munibot_core::{config::Config, greeting::get_greeting_response};
use twitch_irc::message::ServerMessage;

use crate::{
    agent::TwitchAgent,
    bot::MuniBotTwitchIRCClient,
    handler::{TwitchHandlerError, TwitchMessageHandler},
};

/// Twitch greeting handler. Uses munibot_core::greeting for the matching
/// logic, implementing only the Twitch side.
pub struct GreetingHandler;

#[async_trait]
impl TwitchMessageHandler for GreetingHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = if let ServerMessage::Privmsg(m) = message {
            if let Some(response) = get_greeting_response(&m.sender.name, &m.message_text) {
                self.send_twitch_message(client, &m.channel_login, &response)
                    .await?;
                true
            } else {
                false
            }
        } else {
            false
        };

        Ok(handled)
    }
}
