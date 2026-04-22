use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use log::{debug, error, warn};
use munibot_core::{
    db::{DbPool, models::AutoDeleteTimerRow, operations},
    error::MuniBotError as CoreError,
};
use poise::serenity_prelude::{
    Cache, CacheHttp, ChannelId, GuildChannel, GuildId, Mentionable, Message, MessageBuilder,
    MessageId, PartialGuild, Result,
    futures::{StreamExt, stream},
};
use strum::EnumString;
use tokio::{runtime::Handle, sync::Mutex, task::JoinHandle};

use crate::{error::MuniBotError, handlers::logging::LoggingHandler, state::GlobalAccess};

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

        let db_records = operations::get_all_autodelete_timers(global_access.db())
            .await
            .map_err(|e| CoreError::Other(format!("error loading autodelete timers: {e}")))?;

        for row in db_records {
            let channel_id = ChannelId::new(row.channel_id as u64);
            timers.insert(channel_id, AutoDeleteTimer::from_row(row));
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
        let row = AutoDeleteTimerRow {
            channel_id: channel_id.get() as i64,
            guild_id: guild_id.get() as i64,
            duration_secs: duration.as_secs() as i64,
            last_cleaned: DateTime::from_timestamp_nanos(0).naive_utc(),
            last_message_id_cleaned: 1,
            mode: mode.to_db_str().to_owned(),
        };

        let saved = operations::upsert_autodelete_timer(self.access.db(), row)
            .await
            .map_err(|e| anyhow::anyhow!("error saving autodelete timer: {e}"))?;

        let timer = AutoDeleteTimer::from_row(saved);
        self.timers.insert(channel_id, timer);
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
    }

    /// Returns true if there was a timer to delete, and false if nothing was
    /// deleted.
    pub async fn clear_autodelete(
        &mut self,
        guild_id: GuildId,
        channel_id: ChannelId,
    ) -> Result<bool, anyhow::Error> {
        if !self.timers.contains_key(&channel_id) {
            return Ok(false);
        }
        operations::delete_autodelete_timer(self.access.db(), channel_id.get() as i64)
            .await
            .map_err(|e| anyhow::anyhow!("error deleting autodelete timer: {e}"))?;
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
                if timer.should_check()
                    && let Err(e) = timer
                        .clean_now(
                            self.access.as_cache_http(),
                            self.access.db(),
                            self.logging.clone(),
                        )
                        .await
                {
                    error!("timer failed to clean: {e}");
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
                    timer.duration().min(smallest)
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

#[derive(Copy, Clone, Debug, Default, EnumString, poise::ChoiceParameter)]
pub enum AutoDeleteMode {
    /// Deletes any message older than some duration.
    #[name = "always"]
    Always,

    /// Deletes all messages after a channel has not received activity in some
    /// time.
    #[default]
    #[name = "after silence"]
    AfterSilence,
}

impl AutoDeleteMode {
    fn to_db_str(self) -> &'static str {
        match self {
            AutoDeleteMode::Always => "Always",
            AutoDeleteMode::AfterSilence => "AfterSilence",
        }
    }

    fn from_db_str(s: &str) -> Self {
        match s {
            "Always" => AutoDeleteMode::Always,
            _ => AutoDeleteMode::AfterSilence,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutoDeleteTimer {
    row: AutoDeleteTimerRow,
}

struct DeleteMessagesResult {
    deletions: i32,
    failures: i32,
    skipped: i32,
    last_message_deleted: Option<Message>,
}

impl AutoDeleteTimer {
    fn from_row(row: AutoDeleteTimerRow) -> Self {
        Self { row }
    }

    fn channel_id(&self) -> ChannelId {
        ChannelId::new(self.row.channel_id as u64)
    }

    fn guild_id(&self) -> GuildId {
        GuildId::new(self.row.guild_id as u64)
    }

    fn duration(&self) -> Duration {
        Duration::from_secs(self.row.duration_secs as u64)
    }

    fn mode(&self) -> AutoDeleteMode {
        AutoDeleteMode::from_db_str(&self.row.mode)
    }

    fn last_cleaned_utc(&self) -> DateTime<Utc> {
        self.row.last_cleaned.and_utc()
    }

    fn last_message_id_cleaned(&self) -> MessageId {
        MessageId::new(self.row.last_message_id_cleaned as u64)
    }

    /// Returns true if this timer should read messages and decide whether to
    /// clean.
    pub fn should_check(&self) -> bool {
        self.last_cleaned_utc() + self.duration() <= Utc::now()
    }

    /// Cleans channels by deleting messages according to this timer's deletion
    /// mode.
    pub async fn clean_now(
        &mut self,
        cache_http: impl CacheHttp,
        db: &DbPool,
        logging: Arc<Mutex<LoggingHandler>>,
    ) -> Result<(), anyhow::Error> {
        log::debug!(
            "executing cleanup in channel {}",
            self.get_full_name(&cache_http).await
        );

        let (guild, channel) = self.get_guild_channel(&cache_http).await?;

        if let Some(last_message_id) = channel.last_message_id
            && last_message_id.get() != self.last_message_id_cleaned().get()
        {
            // abort if this is an AfterSilence timer that is firing too early
            if let AutoDeleteMode::AfterSilence = self.mode()
                && last_message_id.created_at().to_utc() > Utc::now() - self.duration()
            {
                log::warn!(
                    "autodelete: timer with AfterSilence attempted to fire before its duration \
                     was met"
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
                    .ignore_messages_iter(chopping_block.iter().map(|m| m.id));

                let DeleteMessagesResult {
                    deletions,
                    failures,
                    skipped,
                    last_message_deleted,
                } = self.delete_messages(cache_http_arc, chopping_block).await;

                if failures > 0 {
                    log::warn!(
                        "autodeletion in channel {} (id {}) in {} (id {}): {deletions} deleted, \
                         {skipped} skipped, {failures} failed",
                        channel.name,
                        channel.id,
                        guild.name,
                        guild.id
                    )
                }

                // record last message id if needed
                if let Some(last_deleted_id) = last_message_deleted.map(|m| m.id) {
                    debug!("setting new latest message: {:?}", last_deleted_id);
                    self.row.last_message_id_cleaned = last_deleted_id.get() as i64;
                } else {
                    debug!("not changing latest message id for channel clean-up");
                }

                debug!("cleanup is done");
            }
        } else {
            // probably no messages to clean up, so we can exit now
            debug!(
                "channel {} (id {}) in {} (id {}) has no new messages, so no clean-up will happen \
                 now",
                channel.name, channel.id, guild.name, guild.id
            );
        }

        // update last time this channel was cleaned
        let now_utc: DateTime<Utc> = Utc::now();
        self.row.last_cleaned = now_utc.naive_utc();
        operations::update_autodelete_last_cleaned(
            db,
            self.row.channel_id,
            self.row.last_cleaned,
            self.row.last_message_id_cleaned,
        )
        .await
        .map_err(|e| anyhow::anyhow!("error updating autodelete timer: {e}"))?;

        Ok(())
    }

    async fn get_messages_to_delete(
        &mut self,
        cache_http_arc: &Arc<impl CacheHttp>,
        stream_failures: &Mutex<i32>,
    ) -> Vec<Message> {
        let duration = self.duration();
        self.channel_id()
            .messages_iter(cache_http_arc.http())
            .filter_map(|result| async {
                match result {
                    Ok(m) => {
                        if !m.pinned && m.timestamp.to_utc() <= Utc::now() - duration {
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
            .await
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
                            // message deletion was successful; set latest message deleted
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

    /// Checks the messages in a channel and calculates the next time this timer
    /// should clean.
    pub async fn check_messages(
        &self,
        cache_http: impl CacheHttp,
    ) -> Result<Duration, anyhow::Error> {
        let (guild, channel) = self.get_guild_channel(&cache_http).await?;

        if let Some(last_message_id) = channel.last_message_id {
            let duration = self.duration();
            let duration_to_next_clean = match self.mode() {
                AutoDeleteMode::Always => {
                    // get the oldest message's timestamp
                    let oldest_time = self
                        .channel_id()
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

                    (oldest_time.to_utc() + duration - Utc::now())
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                }
                AutoDeleteMode::AfterSilence => {
                    // use the time of the last message sent plus this timer's duration
                    (last_message_id.created_at().to_utc() + duration - Utc::now())
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
            let duration = self.duration();
            debug!(
                "channel {} (id {}) in {} (id {}) has no messages. we'll check back in after this \
                 timer's duration ({})",
                channel.name,
                channel.id,
                guild.name,
                guild.id,
                humantime::Duration::from(duration)
            );
            Ok(duration)
        }
    }

    async fn get_guild_channel(
        &self,
        cache_http: impl CacheHttp,
    ) -> Result<(PartialGuild, GuildChannel), anyhow::Error> {
        let channel_id = self.channel_id();
        let guild_id = self.guild_id();

        let guild_channel = match cache_http
            .cache()
            .and_then(|cache| cache.guild(guild_id))
            .and_then(|g| g.channels.get(&channel_id).cloned())
        {
            Some(cached_channel) => cached_channel.clone(),
            None => cache_http
                .http()
                .get_channel(channel_id)
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
        self.channel_id()
            .name(cache_http)
            .await
            .unwrap_or_else(|_| "<failed to fetch channel name>".to_string())
    }

    fn guild_name(&self, cache: impl AsRef<Cache>) -> String {
        self.guild_id()
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
