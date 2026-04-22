use async_trait::async_trait;
use munibot_core::greeting::get_greeting_response;
use poise::serenity_prelude::{Context, FullEvent};
use twitch_irc::message::ServerMessage;

use crate::{
    config::Config,
    discord::{
        DiscordFrameworkContext,
        handler::{DiscordEventHandler, DiscordHandlerError},
        utils::display_name_from_message,
    },
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

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

#[async_trait]
impl DiscordEventHandler for GreetingHandler {
    fn name(&self) -> &'static str {
        "greeting"
    }

    async fn handle_discord_event(
        &mut self,
        context: &Context,
        _framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        if let FullEvent::Message { new_message } = event {
            let msg = new_message;
            let author_name = display_name_from_message(msg, &context.http).await;

            if let Some(response) = get_greeting_response(&author_name, &msg.content)
                && msg.author.id != context.cache.current_user().id
            {
                msg.channel_id
                    .say(&context.http, response)
                    .await
                    .map_err(|e| DiscordHandlerError {
                        message: e.to_string(),
                        handler_name: self.name(),
                    })?;
            }
        };

        Ok(())
    }
}
