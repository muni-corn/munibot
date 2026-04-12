use std::time::{Duration, Instant};

use twitch_irc::message::{ReplyToMessage, ServerMessage};

use crate::{
    config::Config,
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

pub struct LiftHandler {
    last_call: Instant,
}

impl LiftHandler {
    pub fn new() -> Self {
        Self {
            last_call: Instant::now() - Duration::from_secs(300),
        }
    }
}

impl Default for LiftHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl TwitchMessageHandler for LiftHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        if let ServerMessage::Privmsg(msg) = message
            && msg.message_text.starts_with("!liftmuni")
            && self.last_call.elapsed().as_secs() > 300
        {
            self.send_twitch_message(client, msg.channel_login(), "nuh uh. not here. muni is streaming right now. you can't do that while he's streaming.").await?;
            self.last_call = Instant::now();

            Ok(true)
        } else {
            Ok(false)
        }
    }
}
