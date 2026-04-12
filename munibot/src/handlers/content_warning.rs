use std::collections::HashSet;

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

pub struct ContentWarningHandler {
    active_warning: Option<String>,
    users_greeted: HashSet<String>,
}

impl ContentWarningHandler {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            active_warning: None,
            users_greeted: HashSet::new(),
        }
    }

    async fn say_user_requested_warning(
        &mut self,
        client: &MuniBotTwitchIRCClient,
        channel: &str,
        addressee: &str,
    ) -> Result<(), TwitchHandlerError> {
        if let Some(warning) = &self.active_warning {
            self.send_twitch_message(client, channel, &format!("hey {addressee}, muni has issued a content/trigger warning for this stream: {warning}. please take care of yourself! it's okay to leave or mute if this content will make you uncomfortable. and you are loved no matter what!")).await
        } else {
            self.send_twitch_message(client, channel, &format!("hey {addressee}, there is no active content/trigger warning in effect. enjoy the stream ^-^ if current conversation is making you uncomfortable, you can use the 'subject change /srs' redeem to change the subject!")).await
        }
    }

    async fn say_streamer_requested_warning(
        &mut self,
        client: &MuniBotTwitchIRCClient,
        channel: &str,
    ) -> Result<(), TwitchHandlerError> {
        if let Some(warning) = &self.active_warning {
            self.send_twitch_message(
                client,
                channel,
                &format!(
                    "hi muni! you have an active content/trigger warning in effect: \"{warning}\""
                ),
            )
            .await
        } else {
            self.send_twitch_message(
                client,
                channel,
                "hi muni! you don't have a content/trigger warning issued right now.",
            )
            .await
        }
    }

    async fn greet_user(
        &mut self,
        client: &MuniBotTwitchIRCClient,
        channel: &str,
        user_name: &str,
    ) -> Result<(), TwitchHandlerError> {
        if !self.users_greeted.contains(user_name)
            && let Some(warning) = &self.active_warning
        {
            self.send_twitch_message(client, channel, &format!("welcome, {user_name}! just so you know, muni has issued a content/trigger warning for this stream: {warning}. please take care of yourself! it's okay to leave or mute if this content will make you uncomfortable. and you are loved no matter what!")).await?;
            self.users_greeted.insert(user_name.to_string());
        }

        Ok(())
    }
}

impl Default for ContentWarningHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TwitchMessageHandler for ContentWarningHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = match message {
            ServerMessage::Privmsg(m) => {
                if let Some(content) = m
                    .message_text
                    .strip_prefix("!cw")
                    .or(m.message_text.strip_prefix("!tw"))
                    .map(|s| s.trim_start())
                {
                    if m.sender.login == m.channel_login {
                        if content.trim().is_empty() {
                            self.say_streamer_requested_warning(client, &m.channel_login)
                                .await?;
                        } else if content == "clear" || content == "reset" {
                            self.active_warning = None;
                            self.send_twitch_message(
                                client,
                                &m.channel_login,
                                "okay! content/trigger warning has been cleared.",
                            )
                            .await?;
                        } else {
                            self.active_warning = Some(content.to_string());
                            self.users_greeted.clear();
                            self.send_twitch_message(client, &m.channel_login, &format!("okay! issued a content/trigger warning with the following reason: \"{content}\"")).await?;
                        }
                    } else {
                        self.say_user_requested_warning(client, &m.channel_login, &m.sender.name)
                            .await?;
                    }
                    true
                } else {
                    self.greet_user(client, &m.channel_login, &m.sender.name)
                        .await?;
                    false
                }
            }
            _ => false,
        };

        Ok(handled)
    }
}
