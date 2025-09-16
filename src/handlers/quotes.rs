use std::env;

use async_trait::async_trait;
use chrono::{DateTime, Local};
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use surrealdb::{
    Surreal,
    engine::remote::ws::{self, Ws},
    opt::auth::Database,
};
use twitch_irc::message::ServerMessage;

use crate::{
    MunibotError,
    config::{Config, DbConfig},
    twitch::{
        agent::TwitchAgent,
        bot::MuniBotTwitchIRCClient,
        handler::{TwitchHandlerError, TwitchMessageHandler},
    },
};

const QUOTE_TABLE: &str = "quote";

/// A quote.
#[derive(Deserialize, Serialize)]
pub struct Quote {
    pub created_at: DateTime<Local>,
    pub quote: String,
    pub invoker: String,
    pub stream_category: String,
    pub stream_title: String,
}

/// A handler for the `!quote` command.
pub struct QuotesHandler {
    db: Surreal<ws::Client>,
}

impl QuotesHandler {
    /// Create a new QuotesHandler, connecting to the database.
    pub async fn new(db_config: &DbConfig) -> Result<Self, MunibotError> {
        dotenv().ok(); // TODO: map to MuniBotError::DotenvError

        let db = Surreal::new::<Ws>(&db_config.url).await?;
        db.signin(Database {
            namespace: "muni_bot",
            database: "muni_bot",
            username: &db_config.user,
            password: &env::var("DATABASE_PASS").expect("expected DATABASE_PASS to be set"),
        })
        .await?;

        Ok(Self { db })
    }

    /// Add a new quote to the database, returning the new count of quotes
    pub async fn add_new_quote(&mut self, new_quote: Quote) -> Result<u32, TwitchHandlerError> {
        self.db
            .create::<Option<Quote>>(QUOTE_TABLE)
            .content(new_quote)
            .await?;

        let count = self
            .db
            .query(format!("SELECT count() FROM {QUOTE_TABLE} GROUP ALL;"))
            .await?
            .take::<Option<u32>>((0, "count"))?
            .unwrap_or(0);

        Ok(count)
    }

    /// Recall a quote from the database and send it in chat.
    pub async fn recall_quote(
        &mut self,
        client: &MuniBotTwitchIRCClient,
        recipient_channel: &str,
        n_requested: Option<i32>,
    ) -> Result<(), TwitchHandlerError> {
        if let Some(n) = n_requested {
            // recall specific quote
            let mut response = self
                .db
                .query(format!(
                    "SELECT * FROM {QUOTE_TABLE}
                     ORDER BY created_at
                     LIMIT 1
                     START $n;",
                ))
                .bind(("n", n - 1))
                .await?;

            if let Some(quote) = response.take::<Option<Quote>>(0)? {
                self.send_twitch_message(
                    client,
                    recipient_channel,
                    &format!(r#"here's quote #{}: "{}""#, n, quote.quote),
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
            // recall random quote
            let mut response = self
                .db
                .query(format!(
                    "SELECT * FROM {QUOTE_TABLE}
                     ORDER BY rand()
                     LIMIT 1;",
                ))
                .await?;

            if let Some(quote) = response.take::<Option<Quote>>(0)? {
                self.send_twitch_message(
                    client,
                    recipient_channel,
                    &format!(r#"random quote: "{}""#, quote.quote),
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
                    let new_quote = Quote {
                        created_at: Local::now(),
                        quote: content.to_string(),
                        invoker: m.sender.id.to_string(),
                        stream_category: channel_info.game_name.take(),
                        stream_title: channel_info.title,
                    };

                    let quote_count = self.add_new_quote(new_quote).await?;
                    self.send_twitch_message(
                        client,
                        &m.channel_login,
                        &format!(
                            "quote #{quote_count} is in! recorded in the muni history books forever"
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
