use async_trait::async_trait;
use twitch_irc::message::ServerMessage;

use crate::{
    config::Config,
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

pub struct SocialsHandler;

#[async_trait]
impl TwitchMessageHandler for SocialsHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = if let ServerMessage::Privmsg(m) = message {
            if m.message_text.trim().starts_with("!discord") {
                if let Some(invite_link) = config.discord.invite_link.as_ref() {
                    self.send_twitch_message(
                        client,
                        &m.channel_login,
                        &format!("join the herd's discord server here! {invite_link}"),
                    )
                    .await?;
                } else {
                    self.send_twitch_message(
                        client,
                        &m.channel_login,
                        "the discord comand is enabled, but no invite link has been configured >.>",
                    )
                    .await?;
                }
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
