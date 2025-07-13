use anyhow::Result;
use async_trait::async_trait;
use log::{error, info, warn};
use tokio::task::JoinHandle;
use twitch_irc::{
    ClientConfig, SecureTCPTransport, TwitchIRCClient, irc, login::StaticLoginCredentials,
    message::ServerMessage,
};

use super::{
    agent::TwitchAgent,
    handler::{TwitchHandlerError, TwitchMessageHandler},
};
use crate::{
    config::Config,
    handlers::{
        TwitchHandlerCollection, affection::AffectionHandler, autoban::AutoBanHandler,
        bonk::BonkHandler, greeting::GreetingHandler, lift::LiftHandler, lurk::LurkHandler,
        magical::MagicalHandler, quotes::QuotesHandler, shoutout::ShoutoutHandler,
        socials::SocialsHandler,
    },
    twitch::tokens::TwitchAuth,
};

pub type MuniBotTwitchIRCClient = TwitchIRCClient<SecureTCPTransport, StaticLoginCredentials>;
pub type MuniBotTwitchIRCError = twitch_irc::Error<SecureTCPTransport, StaticLoginCredentials>;

pub struct TwitchBot {
    auto_ban_handler: AutoBanHandler,
    message_handlers: TwitchHandlerCollection,
}

impl TwitchBot {
    pub async fn new(config: Config) -> Self {
        Self {
            auto_ban_handler: AutoBanHandler,
            message_handlers: vec![
                Box::new(QuotesHandler::new(&config.db).await.unwrap()),
                Box::new(BonkHandler),
                Box::new(SocialsHandler),
                Box::new(LurkHandler),
                Box::new(GreetingHandler),
                Box::new(LiftHandler::new()),
                Box::new(ShoutoutHandler),
                Box::new(AffectionHandler),
                Box::new(MagicalHandler),
            ],
        }
    }

    pub async fn launch(mut self, token: String, bot_config: &Config) -> Result<JoinHandle<()>> {
        let credentials =
            StaticLoginCredentials::new(bot_config.twitch.twitch_user.clone(), Some(token.clone()));
        let cred_config = ClientConfig::new_simple(credentials.clone());
        let twitch_auth = TwitchAuth::new(&bot_config.twitch.twitch_user, &token).await?;
        let agent = TwitchAgent::new(twitch_auth);

        let (mut incoming_messages, irc_client) = MuniBotTwitchIRCClient::new(cred_config);
        irc_client
            .send_message(irc![
                "CAP",
                "REQ",
                ":twitch.tv/tags twitch.tv/commands twitch.tv/membership"
            ])
            .await?;

        // join all the initial channels
        for channel in &bot_config.twitch.initial_channels {
            self.join_channel(channel, &irc_client).await;
        }

        // join our own channel too
        self.join_channel(&bot_config.twitch.twitch_user, &irc_client)
            .await;

        let bot_config_clone = bot_config.clone();
        let handle = tokio::spawn(async move {
            while let Some(message) = incoming_messages.recv().await {
                if let ServerMessage::Notice(notice_msg) = message {
                    if let Some(channel) = notice_msg.channel_login {
                        warn!(
                            "notice received from {}: {}",
                            channel, notice_msg.message_text
                        );
                    } else {
                        warn!("notice received from twitch: {}", notice_msg.message_text);
                    }
                } else if let Err(e) = self
                    .handle_twitch_message(&message, &irc_client, &agent, &bot_config_clone)
                    .await
                {
                    error!("error in twitch message handler! {e}");
                }
            }
        });
        Ok(handle)
    }

    async fn join_channel(&self, channel: &str, client: &MuniBotTwitchIRCClient) {
        // join a channel. this will error if the passed channel login name is
        // malformed.
        if let Err(e) = client.join(channel.to_string()) {
            error!("error joining {}'s twitch channel :( {}", channel, e);
        }
        info!("twitch: joined channel {}", channel);
    }
}

#[async_trait]
impl TwitchMessageHandler for TwitchBot {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        agent: &TwitchAgent,
        config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        if let Err(e) = self
            .auto_ban_handler
            .handle_twitch_message(message, client, agent, config)
            .await
        {
            error!("error in autoban handler at root: {}", e);
        }

        if let ServerMessage::Privmsg(privmsg) = message
            && privmsg.channel_login == "muni_corn"
        {
            for message_handler in self.message_handlers.iter_mut() {
                // try to handle the message. if the handler determines the message was handled,
                // we'll stop
                match message_handler
                    .handle_twitch_message(message, client, agent, config)
                    .await
                {
                    Ok(true) => return Ok(true),
                    Err(e) => error!("error twitch in message handler: {}", e),
                    _ => continue,
                }
            }
        }

        return Ok(false);
    }
}
