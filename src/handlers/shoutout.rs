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

pub struct ShoutoutHandler;

#[async_trait]
impl TwitchMessageHandler for ShoutoutHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        if let ServerMessage::Privmsg(msg) = message {
            // accept either !so or !shoutout
            if let Some(target) = msg
                .message_text
                .strip_prefix("!so ")
                .or_else(|| msg.message_text.strip_prefix("!shoutout "))
                // strip @ from the front of the username if it's there
                .map(|s| s.trim_start_matches('@'))
            {
                let message = format!(
                    "this is a PSA that you NEED to go check out {target} at https://twitch.tv/{target} ! :3 clearly they deserve the shoutout, so go follow them now >:c"
                );

                // send the message
                self.send_twitch_message(client, &msg.channel_login, &message)
                    .await?;

                Ok(true)
            } else if let Some(targets_raw) = msg
                // multi-shoutouts
                .message_text
                .strip_prefix("!mso ")
            {
                let mut message = String::from("go check out these cuties! :3");

                for mut target in targets_raw.split_whitespace() {
                    target = target.trim_start_matches('@');
                    let link = format!(" https://twitch.tv/{}", target);

                    // if the message after adding this link would exceed twitch's character limit
                    // of 500, send the message first and reset it
                    if message.len() + link.len() >= 500 {
                        self.send_twitch_message(client, &msg.channel_login, &message)
                            .await?;
                        message = link.trim().to_string();
                    } else {
                        // otherwise, add the link to the message
                        message.push_str(&link);
                    }
                }

                self.send_twitch_message(client, &msg.channel_login, &message)
                    .await?;

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}
