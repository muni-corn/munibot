use async_trait::async_trait;
use munibot_core::magical::get_magic_message;
use twitch_irc::message::ServerMessage;

use crate::{
    MuniBotError,
    config::Config,
    discord::{
        DiscordCommand, DiscordContext, commands::DiscordCommandProvider,
        utils::display_name_from_command_context,
    },
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

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

/// Check your magicalness today.
#[poise::command(prefix_command, slash_command)]
async fn magical(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    let nick = display_name_from_command_context(ctx).await;

    ctx.say(get_magic_message(&ctx.author().id.to_string(), &nick))
        .await?;

    Ok(())
}

impl DiscordCommandProvider for MagicalHandler {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![magical()]
    }
}
