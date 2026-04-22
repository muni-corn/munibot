use async_trait::async_trait;
use chrono::Local;
use twitch_irc::message::ServerMessage;

use crate::{
    CoreError, MuniBotError,
    config::Config,
    db::{DbPool, operations},
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

/// A handler for the `!quote` and `!addquote` commands.
pub struct QuotesHandler {
    pool: DbPool,
    community_id: i64,
}

impl QuotesHandler {
    /// Creates a new `QuotesHandler`. Looks up (or creates) the community link
    /// for the given Twitch streamer ID to obtain the `community_id` used for
    /// all quote queries.
    pub async fn new(pool: DbPool, twitch_streamer_id: &str) -> Result<Self, MuniBotError> {
        let link = operations::get_or_create_community_link_by_twitch_id(&pool, twitch_streamer_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        Ok(Self {
            pool,
            community_id: link.id,
        })
    }

    /// Adds a new quote to the database, returning the new per-community quote
    /// number.
    async fn add_new_quote(
        &self,
        quote: String,
        invoker: String,
        stream_category: String,
        stream_title: String,
    ) -> Result<i32, TwitchHandlerError> {
        let created_at = Local::now().naive_local();
        let saved = operations::add_quote(
            &self.pool,
            self.community_id,
            created_at,
            quote,
            invoker,
            stream_category,
            stream_title,
        )
        .await
        .map_err(|e| TwitchHandlerError::Other(e.to_string()))?;

        Ok(saved.sequential_id)
    }

    /// Recalls a quote from the database and sends it in chat.
    async fn recall_quote(
        &mut self,
        client: &MuniBotTwitchIRCClient,
        recipient_channel: &str,
        n_requested: Option<i32>,
    ) -> Result<(), TwitchHandlerError> {
        if let Some(n) = n_requested {
            let quote = operations::get_quote_by_number(&self.pool, self.community_id, n)
                .await
                .map_err(|e| TwitchHandlerError::Other(e.to_string()))?;

            if let Some(q) = quote {
                self.send_twitch_message(
                    client,
                    recipient_channel,
                    &format!(r#"here's quote #{}: "{}""#, q.sequential_id, q.quote),
                )
                .await
            } else {
                self.send_twitch_message(
                    client,
                    recipient_channel,
                    &format!("quote #{n} not found :("),
                )
                .await
            }
        } else {
            let quote = operations::get_random_quote(&self.pool, self.community_id)
                .await
                .map_err(|e| TwitchHandlerError::Other(e.to_string()))?;

            if let Some(q) = quote {
                self.send_twitch_message(
                    client,
                    recipient_channel,
                    &format!(r#"random quote: "{}""#, q.quote),
                )
                .await
            } else {
                self.send_twitch_message(client, recipient_channel, "no quotes found :(")
                    .await
            }
        }
    }
}

#[async_trait]
impl TwitchMessageHandler for QuotesHandler {
    async fn handle_twitch_message(
        &mut self,
        message: &ServerMessage,
        client: &MuniBotTwitchIRCClient,
        agent: &TwitchAgent,
        _config: &Config,
    ) -> Result<bool, TwitchHandlerError> {
        let handled = if let ServerMessage::Privmsg(m) = message {
            if let Some(content) = m.message_text.strip_prefix("!addquote").map(str::trim) {
                if content.is_empty() {
                    self.send_twitch_message(
                        client,
                        &m.channel_login,
                        "i can't add an empty quote!",
                    )
                    .await?;
                } else if let Some(channel_info) = agent.get_channel_info(&m.channel_id).await? {
                    let quote_number = self
                        .add_new_quote(
                            content.to_string(),
                            m.sender.id.to_string(),
                            channel_info.game_name.take(),
                            channel_info.title,
                        )
                        .await?;
                    self.send_twitch_message(
                        client,
                        &m.channel_login,
                        &format!(
                            "quote #{quote_number} is in! recorded in the muni history books \
                             forever"
                        ),
                    )
                    .await?;
                }

                true
            } else if let Some(content) = m.message_text.strip_prefix("!quote").map(str::trim) {
                if content.is_empty() {
                    // recall a random quote
                    self.recall_quote(client, &m.channel_login, None).await?;
                } else if let Ok(n) = content.parse::<i32>() {
                    self.recall_quote(client, &m.channel_login, Some(n)).await?;
                } else if content.len() >= 3 {
                    // TODO: recall a quote that matches the content
                }

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
