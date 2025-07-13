use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use async_trait::async_trait;
use chrono::Local;
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

impl MagicalHandler {
    fn get_magic_amount(user_id: &str) -> u8 {
        // we determine a user's magicalness based on the current date and their user
        // id.
        let today = Local::now().date_naive();

        // hash the value
        let mut hash_state = DefaultHasher::new();
        (today, user_id).hash(&mut hash_state);
        let hashed = hash_state.finish();

        // a number between 0 and 100
        let x = hashed % 101;

        // give a cubic-interpolated value between 1 and 100, favoring higher numbers,
        // without floating point arithmetic :>
        ((100u64.pow(3) - x * x * x) / (100 * 100)) as u8
    }

    fn get_message(user_id: &str, user_display_name: &str) -> String {
        let magic_amount = Self::get_magic_amount(user_id);
        let suffix = match magic_amount {
            x if x <= 1 => ". you can have some of my magic~ :3 <3",
            x if x < 25 => ". sounds like a good day for some self care <3",
            69 => ". nice ;3",
            x if x < 75 => ".",
            100 => "!! wow :3",
            _ => "!",
        };
        format!("{user_display_name} is {magic_amount}% magical today{suffix}")
    }
}

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
            ServerMessage::Privmsg(msg) => {
                if msg.message_text.starts_with("!magical") {
                    self.send_twitch_message(
                        client,
                        &msg.channel_login,
                        &Self::get_message(&msg.sender.id, &msg.sender.name),
                    )
                    .await?;
                    true
                } else {
                    false
                }
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

    ctx.say(MagicalHandler::get_message(
        &ctx.author().id.to_string(),
        &nick,
    ))
    .await?;

    Ok(())
}

impl DiscordCommandProvider for MagicalHandler {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![magical()]
    }
}
