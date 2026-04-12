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

#[cfg(test)]
mod tests {
    use super::GreetingHandler;

    #[test]
    fn test_hi_munibot_matches() {
        let result = GreetingHandler::get_greeting_message("alice", "hi munibot");
        assert!(result.is_some(), "expected greeting for 'hi munibot'");
    }

    #[test]
    fn test_hello_munibot_matches() {
        let result = GreetingHandler::get_greeting_message("alice", "hello munibot");
        assert!(result.is_some(), "expected greeting for 'hello munibot'");
    }

    #[test]
    fn test_hey_munibot_matches() {
        let result = GreetingHandler::get_greeting_message("alice", "hey munibot");
        assert!(result.is_some(), "expected greeting for 'hey munibot'");
    }

    #[test]
    fn test_hewwo_munibot_matches() {
        let result = GreetingHandler::get_greeting_message("alice", "hewwo munibot");
        assert!(result.is_some(), "expected greeting for 'hewwo munibot'");
    }

    #[test]
    fn test_henlo_munibot_matches() {
        let result = GreetingHandler::get_greeting_message("alice", "henlo munibot");
        assert!(result.is_some(), "expected greeting for 'henlo munibot'");
    }

    #[test]
    fn test_case_insensitive_match() {
        let result = GreetingHandler::get_greeting_message("alice", "HI MUNIBOT");
        assert!(result.is_some(), "expected greeting to be case-insensitive");
    }

    #[test]
    fn test_words_between_hi_and_munibot() {
        // regex allows words between the greeting and "munibot"
        let result = GreetingHandler::get_greeting_message("alice", "hi there munibot!");
        assert!(result.is_some(), "expected greeting with words in between");
    }

    #[test]
    fn test_no_match_without_munibot() {
        let result = GreetingHandler::get_greeting_message("alice", "hi everyone");
        assert!(result.is_none(), "expected no greeting without 'munibot'");
    }

    #[test]
    fn test_no_match_unrelated_message() {
        let result = GreetingHandler::get_greeting_message("alice", "how are you doing?");
        assert!(
            result.is_none(),
            "expected no greeting for unrelated message"
        );
    }

    #[test]
    fn test_no_match_empty_string() {
        let result = GreetingHandler::get_greeting_message("alice", "");
        assert!(result.is_none(), "expected no greeting for empty message");
    }

    #[test]
    fn test_response_contains_username() {
        let result = GreetingHandler::get_greeting_message("alice", "hi munibot");
        let msg = result.expect("expected a greeting message");
        assert!(
            msg.contains("alice"),
            "expected response to contain username, got '{msg}'"
        );
    }

    #[test]
    fn test_different_users_get_their_name() {
        let result = GreetingHandler::get_greeting_message("bobcat", "hello munibot");
        let msg = result.expect("expected a greeting message");
        assert!(
            msg.contains("bobcat"),
            "expected response to contain 'bobcat', got '{msg}'"
        );
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
