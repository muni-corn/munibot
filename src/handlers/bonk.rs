use async_trait::async_trait;
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use twitch_irc::message::ServerMessage;

use crate::{
    config::Config,
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

pub struct BonkHandler;

#[async_trait]
impl TwitchMessageHandler for BonkHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        _agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = if let ServerMessage::Privmsg(m) = message {
            if let Some(target) = m.message_text.trim().strip_prefix("!bonk ") {
                // pick a template and craft message by replacing all {target}s with the
                // message's arguments
                let mut rng = StdRng::from_entropy();
                let message = BONK_TEMPLATES
                    .choose(&mut rng)
                    .unwrap()
                    .replace("{target}", target);

                // and send!
                self.send_twitch_message(client, &m.channel_login, &message)
                    .await
                    .unwrap();

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

const BONK_TEMPLATES: [&str; 10] = [
    "{target}, stop being naughty BOP",
    "sorry about this, {target} BOP",
    "{target} >:( BOP",
    "*sigh* this will only hurt a little, {target} BOP",
    "{target} needs a bonk?? BOP",
    "surely you saw this coming, {target}? BOP",
    "here you go, {target} BOP",
    "don't move, {target} BOP",
    "bad {target}, bad! BOP",
    "sounds like you've been naughty, {target} BOP",
];
