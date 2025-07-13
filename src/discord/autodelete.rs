use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use log::{debug, error, warn};
use poise::serenity_prelude::{
    Cache, CacheHttp, ChannelId, GuildChannel, GuildId, Mentionable, Message, MessageBuilder,
    MessageId, PartialGuild, Result,
    futures::{StreamExt, stream},
};
use serde::{Deserialize, Serialize};
use strum::EnumString;
use surrealdb::{Connection, RecordId, Surreal};
use tokio::{runtime::Handle, sync::Mutex, task::JoinHandle};

use super::state::GlobalAccess;
use crate::{
    MuniBotError,
    handlers::logging::LoggingHandler,
    handlers::logging::{LoggingHandler, PauseType},
};

const TABLE_NAME: &str = "autodelete_timer";

#[derive(Debug)]
pub struct AutoDeleteHandler {
    timers: HashMap<ChannelId, AutoDeleteTimer>,
    access: GlobalAccess,
    logging: Arc<Mutex<LoggingHandler>>,
}

impl AutoDeleteHandler {
    const MAXIMUM_WAIT_TIME: Duration = Duration::from_mins(30);
    pub const MINIMUM_TIMER_DURATION: Duration = Duration::from_hours(1);

    pub async fn new(
        global_access: GlobalAccess,
        logging: Arc<Mutex<LoggingHandler>>,
    ) -> Result<Self, MuniBotError> {
        let mut timers = HashMap::new();

        let db_records: Vec<AutoDeleteTimer> = global_access
            .db()
            .query("SELECT * FROM type::table($table)")
            .bind(("table", TABLE_NAME))
            .await?
            .take(0)?;

        for record in db_records {
            timers.insert(record.data.channel_id, record);
        }

        debug!("loaded timers: {:?}", timers);

        Ok(Self {
            timers,
            access: global_access,
            logging,
        })
    }

    pub async fn set_autodelete(
        &mut self,
        guild_id: GuildId,
        channel_id: ChannelId,
        duration: Duration,
        mode: AutoDeleteMode,
    ) -> Result<(), anyhow::Error> {
        let new_maybe: Option<AutoDeleteTimer> = self
            .access
            .db()
            .upsert((TABLE_NAME, channel_id.get() as i64))
            .content(PartialAutoDeleteTimer {
                guild_id,
                channel_id,
                duration,
                mode,
                last_cleaned: DateTime::from_timestamp_nanos(0),
                last_message_id_cleaned: 1.into(),
            })
            .await?;

        if let Some(new) = new_maybe {
            self.timers.insert(channel_id, new);
            debug!("new timers map: {:?}", self.timers);

            // build log message
            let mut msg = MessageBuilder::default();
            msg.push("messages in ")
                .push(channel_id.mention().to_string())
                .push("will be deleted ");

            match mode {
                AutoDeleteMode::Always => msg
                    .push("when they are older than ")
                    .push_bold(humantime::format_duration(duration).to_string())
                    .push('.'),

                AutoDeleteMode::AfterSilence => msg
                    .push("after ")
                    .push_bold(humantime::format_duration(duration).to_string())
                    .push(" of silence."),
            };

            self.logging
                .lock()
                .await
                .send_simple_log(guild_id, "autodelete timer set", &msg.build())
                .await?;
            Ok(())
        } else {
            log::warn!(
                "tried to save an autodelete timer (channel id {channel_id}) but it doesn't seem to have been persisted in the database"
            );
            Err(anyhow::anyhow!(
                "tried to set an autodelete timer, but i don't think i was able to save it..."
            ))
        }
    }

    /// returns true if there was a timer to delete, and false if nothing was
    /// deleted.
    pub async fn clear_autodelete(
        &mut self,
        guild_id: GuildId,
        channel_id: ChannelId,
    ) -> Result<bool, anyhow::Error> {
        if !self.timers.contains_key(&channel_id) {
            return Ok(false);
        }
        let _: Option<AutoDeleteTimer> = self
            .access
            .db()
            .delete((TABLE_NAME, channel_id.get() as i64))
            .await?;
        self.timers.remove(&channel_id);

        self.logging
            .lock()
            .await
            .send_simple_log(
                guild_id,
                "autodelete timer removed",
                &format!("for channel {}", channel_id.mention()),
            )
            .await?;

        Ok(true)
    }

    pub async fn fire_due_timers(&mut self) -> Result<(), anyhow::Error> {
        stream::iter(self.timers.values_mut())
            .for_each_concurrent(3, |timer| async {
                debug!(
                    "checking if we should fire timer for {} in {}",
                    timer.channel_name(self.access.as_cache_http()).await,
                    timer.guild_name(self.access.cache())
                );
                if timer.should_check() {
                    if let Err(e) = timer
                        .clean_now(
                            self.access.as_cache_http(),
                            self.access.db().clone(),
                            self.logging.clone(),
                        )
                        .await
                    {
                        error!("timer failed to clean: {e}");
                    }
                }
            })
            .await;
        Ok(())
    }

    pub async fn get_next_fire(&mut self) -> Duration {
        let cache_http = self.access.as_cache_http();
        stream::iter(self.timers.values())
            .fold(Self::MAXIMUM_WAIT_TIME, |smallest, timer| async move {
                if timer.should_check() {
                    debug!(
                        "timer for {} is being checked",
                        timer.get_full_name(cache_http).await
                    );
                    match timer.check_messages(cache_http).await {
                        Ok(d) => d.min(smallest),
                        Err(e) => {
                            log::error!("couldn't check messages for autodelete timer: {e}");
                            smallest
                        }
                    }
                } else {
                    debug!(
                        "timer for {} should not be checked",
                        timer.get_full_name(cache_http).await
                    );
                    timer.data.duration.min(smallest)
                }
            })
            .await
    }

    pub fn start(this: Arc<Mutex<Self>>) -> JoinHandle<!> {
        tokio::spawn(async move {
            loop {
                debug!("starting iteration of autodelete loop");
                let sleep_time = {
                    let mut locked = this.lock().await;
                    locked.get_next_fire().await
                };
                debug!(
                    "sleeping until next check in {}",
                    humantime::format_duration(sleep_time)
                );
                tokio::time::sleep(sleep_time).await;
                debug!("firing overdue timers!");
                if let Err(e) = this.lock().await.fire_due_timers().await {
                    error!("autodelete failed when firing timers: {e}");
                }
            }
        })
    }
}

#[derive(
    Copy, Clone, Debug, Default, Deserialize, Serialize, EnumString, poise::ChoiceParameter,
)]
pub enum AutoDeleteMode {
    /// deletes any message older than some duration.
    #[name = "always"]
    Always,

    /// deletes all messages after a channel has not received activity in some
    /// time.
    #[default]
    #[name = "after silence"]
    AfterSilence,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub struct PartialAutoDeleteTimer {
    /// id of the channel this timer runs in.
    channel_id: ChannelId,

    /// guild this timer runs in.
    guild_id: GuildId,

    #[serde(with = "humantime_serde")]
    duration: Duration,

    last_cleaned: DateTime<Utc>,

    #[serde(default)]
    last_message_id_cleaned: MessageId,

    mode: AutoDeleteMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoDeleteTimer {
    id: RecordId,

    #[serde(flatten)]
    data: PartialAutoDeleteTimer,
}

struct DeleteMessagesResult {
    deletions: i32,
    failures: i32,
    skipped: i32,
    last_message_deleted: Option<Message>,
}

impl AutoDeleteTimer {
    /// returns true if this timer should read messages and decide whether to
    /// clean
    pub fn should_check(&self) -> bool {
        self.data.last_cleaned + self.data.duration <= Utc::now()
    }

    /// cleans channels by deleting messages according to this timer's deletion
    /// mode.
    pub async fn clean_now<C: Connection>(
        &mut self,
        cache_http: impl CacheHttp,
        db: Surreal<C>,
        logging: Arc<Mutex<LoggingHandler>>,
    ) -> Result<(), anyhow::Error> {
        log::debug!(
            "executing cleanup in channel {}",
            self.get_full_name(&cache_http).await
        );

        let (guild, channel) = self.get_guild_channel(&cache_http).await?;

        if let Some(last_message_id) = channel.last_message_id
            && last_message_id.get() != self.data.last_message_id_cleaned.get()
        {
            // abort if this is an AfterSilence timer that is firing too early
            if let AutoDeleteMode::AfterSilence = self.data.mode
                && last_message_id.created_at().to_utc() > Utc::now() - self.data.duration
            {
                log::warn!(
                    "autodelete: timer with AfterSilence attempted to fire before its duration was met"
                );
                return Ok(());
            }

            // collect all messages older than this timer's duration
            log::debug!(
                "{} is collecting messages to delete for autodeletion",
                self.get_full_name(&cache_http).await
            );
            let cache_http_arc = Arc::new(cache_http);
            let stream_failures = Mutex::new(0);
            let chopping_block = self
                .get_messages_to_delete(&cache_http_arc, &stream_failures)
                .await;

            // log streaming failures if needed
            let stream_failures = stream_failures.into_inner();
            if stream_failures > 0 {
                warn!(
                    "{} couldn't stream {stream_failures} messages for autodeletion",
                    self.get_full_name(cache_http_arc.clone()).await
                );
            }

            // abort if there aren't any messages to delete
            if chopping_block.is_empty() {
                debug!(
                    "couldn't collect any messages to delete for {}",
                    self.get_full_name(cache_http_arc).await
                );
            } else {
                // first, pause logging
                logging
                    .lock()
                    .await
                    .set_pauses(
                        self.data.channel_id,
                        &[PauseType::MessageDeleteBulk, PauseType::MessageDelete],
                        "munibot is cleaning up messages because an autodelete timer has fired",
                    )
                    .await;

                let DeleteMessagesResult {
                    deletions,
                    failures,
                    skipped,
                    last_message_deleted,
                } = self.delete_messages(cache_http_arc, chopping_block).await;

                logging
                    .lock()
                    .await
                    .clear_pauses(self.data.channel_id)
                    .await;

                if failures > 0 {
                    log::warn!(
                        "autodeletion in channel {} (id {}) in {} (id {}): {deletions} deleted, {skipped} skipped, {failures} failed",
                        channel.name,
                        channel.id,
                        guild.name,
                        guild.id
                    )
                }

                // record last message id if needed
                if let Some(last_deleted_id) = last_message_deleted.map(|m| m.id) {
                    debug!("setting new latest message: {:?}", last_deleted_id);
                    self.data.last_message_id_cleaned = last_deleted_id
                } else {
                    debug!("not changing latest message id for channel clean-up",);
                }

                debug!("cleanup is done");
            }
        } else {
            // probably no messages to clean up, so we can exit now
            debug!(
                "channel {} (id {}) in {} (id {}) has no new messages, so no clean-up will happen now",
                channel.name, channel.id, guild.name, guild.id
            );
        }

        // update last time this channel was cleaned
        self.data.last_cleaned = Utc::now();
        let _: Option<Self> = db
            .upsert((TABLE_NAME, self.data.channel_id.get() as i64))
            .merge(LastCleanedUpdate {
                last_cleaned: self.data.last_cleaned,
                last_message_id_cleaned: self.data.last_message_id_cleaned,
            })
            .await?;

        Ok(())
    }

    async fn get_messages_to_delete(
        &mut self,
        cache_http_arc: &Arc<impl CacheHttp>,
        stream_failures: &Mutex<i32>,
    ) -> Vec<Message> {
        let chopping_block: Vec<Message> = self
            .data
            .channel_id
            .messages_iter(cache_http_arc.http())
            .filter_map(|result| async {
                match result {
                    Ok(m) => {
                        if !m.pinned && m.timestamp.to_utc() <= Utc::now() - self.data.duration {
                            Some(m)
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        (*stream_failures.lock().await) += 1;
                        debug!("failed to stream message for cleanup: {e}");
                        None
                    }
                }
            })
            .collect()
            .await;
        chopping_block
    }

    async fn delete_messages(
        &self,
        cache_http_arc: Arc<impl CacheHttp>,
        chopping_block: Vec<Message>,
    ) -> DeleteMessagesResult {
        stream::iter(chopping_block)
            .fold(
                DeleteMessagesResult {
                    deletions: 0,
                    failures: 0,
                    skipped: 0,
                    last_message_deleted: None,
                },
                |mut stats, m| {
                    let cache_http = cache_http_arc.clone();
                    async move {
                        if let Err(e) = m.delete(cache_http).await {
                            // log the deletion failure
                            log::error!("autodelete failed to delete a message: {e}");
                            stats.failures += 1;
                            stats
                        } else {
                            // message deletion was successful
                            // set latest message deleted
                            if let Some(latest_deleted_message) = stats.last_message_deleted {
                                stats.last_message_deleted =
                                    if m.timestamp >= latest_deleted_message.timestamp {
                                        Some(m)
                                    } else {
                                        Some(latest_deleted_message)
                                    };
                            };

                            stats.deletions += 1;
                            stats
                        }
                    }
                },
            )
            .await
    }

    /// checks the messages in a channel and calculates the next time this timer
    /// should clean.
    pub async fn check_messages(
        &self,
        cache_http: impl CacheHttp,
    ) -> Result<Duration, anyhow::Error> {
        let (guild, channel) = self.get_guild_channel(&cache_http).await?;

        if let Some(last_message_id) = channel.last_message_id {
            let duration_to_next_clean = match self.data.mode {
                AutoDeleteMode::Always => {
                    // get the oldest message's timestamp
                    // there's gotta be a better way to do this, right?
                    let oldest_time = self
                        .data
                        .channel_id
                        .messages_iter(&cache_http.http())
                        .filter_map(|r| async {
                            match r {
                                Ok(m) => Some(m.timestamp),
                                Err(e) => {
                                    log::warn!("error when streaming message to check timer: {e}");
                                    None
                                }
                            }
                        })
                        .fold(
                            last_message_id.created_at(),
                            |acc, t| async move { t.min(acc) },
                        )
                        .await;

                    (oldest_time.to_utc() + self.data.duration - Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                }
                AutoDeleteMode::AfterSilence => {
                    // use the time of the last message sent plus this timer's duration
                    (last_message_id.created_at().to_utc() + self.data.duration - Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                }
            };

            debug!(
                "next clean for {} is in {}",
                self.get_full_name(cache_http).await,
                humantime::format_duration(duration_to_next_clean)
            );
            Ok(duration_to_next_clean)
        } else {
            // probably no messages to clean up, so we can exit now
            debug!(
                "channel {} (id {}) in {} (id {}) has no messages. we'll check back in after this timer's duration ({})",
                channel.name,
                channel.id,
                guild.name,
                guild.id,
                humantime::Duration::from(self.data.duration)
            );
            Ok(self.data.duration)
        }
    }

    async fn get_guild_channel(
        &self,
        cache_http: impl CacheHttp,
    ) -> Result<(PartialGuild, GuildChannel), anyhow::Error> {
        let guild_channel = match cache_http
            .cache()
            .and_then(|cache| cache.guild(self.data.guild_id))
            .and_then(|g| g.channels.get(&self.data.channel_id).cloned())
        {
            Some(cached_channel) => cached_channel.clone(),
            None => cache_http
                .http()
                .get_channel(self.data.channel_id)
                .await?
                .guild()
                .ok_or(anyhow::anyhow!("provided channel is not in a guild"))?,
        };

        let guild = match cache_http
            .cache()
            .and_then(|cache| guild_channel.guild(cache))
        {
            Some(cached_guild) => cached_guild.clone().into(),
            None => {
                Handle::current().block_on(cache_http.http().get_guild(guild_channel.guild_id))?
            }
        };

        Ok((guild, guild_channel))
    }

    async fn channel_name(&self, cache_http: impl CacheHttp) -> String {
        self.data
            .channel_id
            .name(cache_http)
            .await
            .unwrap_or_else(|_| "<failed to fetch channel name>".to_string())
    }

    fn guild_name(&self, cache: impl AsRef<Cache>) -> String {
        self.data
            .guild_id
            .name(cache)
            .unwrap_or_else(|| "<no guild name in cache>".to_string())
    }

    async fn get_full_name(&self, cache_http: impl CacheHttp) -> String {
        let guild_name = if let Some(cache) = cache_http.cache() {
            self.guild_name(cache)
        } else {
            "<no cache for guild name>".to_string()
        };

        let channel_name = self.channel_name(cache_http).await;

        format!("#{channel_name} in \"{guild_name}\"")
    }
}

#[derive(Deserialize, Serialize)]
struct LastCleanedUpdate {
    last_cleaned: DateTime<Utc>,
    last_message_id_cleaned: MessageId,
}
