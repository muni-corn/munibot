use async_trait::async_trait;
use once_cell::sync::Lazy;
use poise::serenity_prelude::{Context, FullEvent};
use rand::seq::SliceRandom;
use regex::Regex;
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

static HI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:hi+|hey+|hello+|howdy+|sup+|heww?o+|henlo+)\b.*\bmuni.?bot\b").unwrap()
});

impl GreetingHandler {
    /// Returns a greeting message if applicable, or None if not to keep quiet.
    fn get_greeting_message(user_name: &str, message_text: &str) -> Option<String> {
        if HI_REGEX.is_match(message_text) {
            // send a hi message back
            // pick a template
            let mut rng = rand::thread_rng();
            let greeting = HELLO_TEMPLATES
                .choose(&mut rng)
                .unwrap()
                .replace("{name}", user_name);

            Some(greeting)
        } else {
            None
        }
    }
}

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
            if let Some(response) = Self::get_greeting_message(&m.sender.name, &m.message_text) {
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

            if let Some(response) = Self::get_greeting_message(&author_name, &msg.content)
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

const HELLO_TEMPLATES: [&str; 11] = [
    "hi, {name}!<3",
    "hello, {name}! happy to see you!",
    "hey {name}:)",
    "hi {name}!! how are ya?",
    "{name}!! how are you doing?",
    "heyyy {name} uwu",
    "hi {name}! it's good to see you! :3",
    "{name} helloooooo:)",
    "hiiiii {name}",
    "hi {name}<3",
    "hi {name}! you look wonderful today ;3",
];
