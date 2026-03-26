use chrono::{DateTime, Local, SubsecRound, Utc};
use log::{info, warn};
use serde::Deserialize;
use surrealdb::{Connection, Surreal};

use crate::{
    MuniBotError,
    db::{
        DbPool,
        models::{
            AutoDeleteTimerRow, GuildConfig, GuildPayout, GuildWallet, NewCommunityLink,
            NewGuildPayout, NewGuildWallet, Quote,
        },
        operations,
        schema::{
            autodelete_timers, community_links, guild_configs, guild_payouts, guild_wallets, quotes,
        },
    },
};

/// The Twitch streamer ID used for the default community that existing quotes
/// will be migrated into.
const DEFAULT_TWITCH_STREAMER_ID: &str = "590712444";

// SurrealDB record shapes (read-only, used only during migration)

#[derive(Debug, Deserialize)]
struct SurrealLoggingChannel {
    // surrealdb record id key is the guild id
    id: surrealdb::RecordId,
    channel_id: i64,
}

#[derive(Debug, Deserialize)]
struct SurrealAutoDeleteTimer {
    guild_id: i64,
    channel_id: i64,
    #[serde(with = "humantime_serde")]
    duration: std::time::Duration,
    last_cleaned: DateTime<Utc>,
    #[serde(default)]
    last_message_id_cleaned: i64,
    mode: String,
}

#[derive(Debug, Deserialize)]
struct SurrealGuildWallet {
    guild_id: i64,
    user_id: i64,
    balance: u64,
}

#[derive(Debug, Deserialize)]
struct SurrealGuildPayout {
    guild_id: i64,
    user_id: i64,
    balance: u64,
    last_payout: DateTime<Local>,
}

#[derive(Debug, Deserialize)]
struct SurrealQuote {
    created_at: DateTime<Utc>,
    quote: String,
    invoker: String,
    stream_category: String,
    stream_title: String,
}

/// Verifies data completeness and accuracy after migration.
///
/// Performs four categories of checks against all migrated tables:
///
/// 1. **Spot-check field values:** loads all MySQL rows and compares key fields
///    against the in-memory SurrealDB source data.
/// 2. **Aggregate checksums:** compares numeric column sums (balance, duration)
///    between source and destination.
/// 3. **Null/empty field validation:** warns on rows with empty strings or zero
///    values in fields that should always be populated.
/// 4. **Referential integrity:** verifies every migrated quote references the
///    correct `community_id`.
///
/// All failures are logged as warnings; this function never returns an error.
/// Returns `true` if every check passed, `false` if any mismatch was found.
async fn verify_migration_data(
    pool: &DbPool,
    surreal_log_channels: &[SurrealLoggingChannel],
    surreal_timers: &[SurrealAutoDeleteTimer],
    surreal_wallets: &[SurrealGuildWallet],
    surreal_payouts: &[SurrealGuildPayout],
    surreal_quotes: &[SurrealQuote],
    default_community_id: i64,
) -> bool {
    use std::collections::HashMap;

    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    let mut all_ok = true;

    // log a warning and mark the run as failed
    macro_rules! data_warn {
        ($($arg:tt)*) => {{
            log::warn!($($arg)*);
            all_ok = false;
        }};
    }

    // =========================================================================
    // guild_configs
    // =========================================================================
    info!("migration verification: checking guild_configs");
    let mysql_configs: Option<Vec<GuildConfig>> = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        match guild_configs::table
            .select(GuildConfig::as_select())
            .load::<GuildConfig>(&mut conn)
            .await
        {
            Ok(rows) => Some(rows),
            Err(e) => {
                data_warn!("migration verification: could not load guild_configs: {e}");
                None
            }
        }
    };

    if let Some(ref configs) = mysql_configs {
        let mysql_map: HashMap<i64, Option<i64>> = configs
            .iter()
            .map(|r| (r.guild_id, r.logging_channel))
            .collect();

        // spot-check: logging_channel matches source
        for row in surreal_log_channels {
            let guild_id = match i64::try_from(row.id.key().clone()) {
                Ok(n) => n,
                // non-numeric ids were skipped during migration; skip here too
                Err(_) => continue,
            };
            match mysql_map.get(&guild_id) {
                None => data_warn!(
                    "migration verification: guild_configs: guild {} missing in mysql",
                    guild_id
                ),
                Some(&ch) => {
                    if ch != Some(row.channel_id) {
                        data_warn!(
                            "migration verification: guild_configs: guild {} logging_channel \
                             mismatch (expected {:?}, got {:?})",
                            guild_id,
                            Some(row.channel_id),
                            ch
                        );
                    }
                }
            }
        }

        // null check: all source rows had a channel_id, so NULL is unexpected
        let null_count = configs
            .iter()
            .filter(|r| r.logging_channel.is_none())
            .count();
        if null_count > 0 {
            data_warn!(
                "migration verification: guild_configs: {null_count} rows have NULL \
                 logging_channel"
            );
        } else {
            info!("migration verification: guild_configs field values ok");
        }
    }

    // =========================================================================
    // autodelete_timers
    // =========================================================================
    info!("migration verification: checking autodelete_timers");
    let mysql_timers: Option<Vec<AutoDeleteTimerRow>> = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        match autodelete_timers::table
            .select(AutoDeleteTimerRow::as_select())
            .load::<AutoDeleteTimerRow>(&mut conn)
            .await
        {
            Ok(rows) => Some(rows),
            Err(e) => {
                data_warn!("migration verification: could not load autodelete_timers: {e}");
                None
            }
        }
    };

    if let Some(ref timers) = mysql_timers {
        let mysql_map: HashMap<i64, &AutoDeleteTimerRow> =
            timers.iter().map(|r| (r.channel_id, r)).collect();

        // spot-check: duration_secs, guild_id, mode
        for row in surreal_timers {
            match mysql_map.get(&row.channel_id) {
                None => data_warn!(
                    "migration verification: autodelete_timers: channel {} missing in mysql",
                    row.channel_id
                ),
                Some(&m) => {
                    let expected_secs = row.duration.as_secs() as i64;
                    if m.duration_secs != expected_secs {
                        data_warn!(
                            "migration verification: autodelete_timers: channel {} \
                             duration_secs mismatch (expected {expected_secs}, got {})",
                            row.channel_id,
                            m.duration_secs
                        );
                    }
                    if m.guild_id != row.guild_id {
                        data_warn!(
                            "migration verification: autodelete_timers: channel {} guild_id \
                             mismatch (expected {}, got {})",
                            row.channel_id,
                            row.guild_id,
                            m.guild_id
                        );
                    }
                    if m.mode != row.mode {
                        data_warn!(
                            "migration verification: autodelete_timers: channel {} mode \
                             mismatch (expected {:?}, got {:?})",
                            row.channel_id,
                            row.mode,
                            m.mode
                        );
                    }
                }
            }
        }

        // aggregate checksum: sum of duration_secs
        let expected_dur_sum: i64 = surreal_timers
            .iter()
            .map(|r| r.duration.as_secs() as i64)
            .sum();
        let actual_dur_sum: i64 = timers.iter().map(|r| r.duration_secs).sum();
        if expected_dur_sum != actual_dur_sum {
            data_warn!(
                "migration verification: autodelete_timers: duration_secs sum mismatch \
                 (expected {expected_dur_sum}, got {actual_dur_sum})"
            );
        } else {
            info!(
                "migration verification: autodelete_timers duration_secs sum ok ({actual_dur_sum})"
            );
        }

        // empty-value checks
        let zero_dur = timers.iter().filter(|r| r.duration_secs == 0).count();
        if zero_dur > 0 {
            data_warn!(
                "migration verification: autodelete_timers: {zero_dur} rows have \
                 duration_secs = 0"
            );
        }
        let empty_mode = timers.iter().filter(|r| r.mode.is_empty()).count();
        if empty_mode > 0 {
            data_warn!(
                "migration verification: autodelete_timers: {empty_mode} rows have empty mode"
            );
        }
    }

    // =========================================================================
    // guild_wallets
    // =========================================================================
    info!("migration verification: checking guild_wallets");
    let mysql_wallets: Option<Vec<GuildWallet>> = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        match guild_wallets::table
            .select(GuildWallet::as_select())
            .load::<GuildWallet>(&mut conn)
            .await
        {
            Ok(rows) => Some(rows),
            Err(e) => {
                data_warn!("migration verification: could not load guild_wallets: {e}");
                None
            }
        }
    };

    if let Some(ref wallets) = mysql_wallets {
        let mysql_map: HashMap<(i64, i64), u64> = wallets
            .iter()
            .map(|r| ((r.guild_id, r.user_id), r.balance))
            .collect();

        // spot-check: balance per (guild_id, user_id)
        for row in surreal_wallets {
            match mysql_map.get(&(row.guild_id, row.user_id)) {
                None => data_warn!(
                    "migration verification: guild_wallets: (guild={}, user={}) missing in mysql",
                    row.guild_id,
                    row.user_id
                ),
                Some(&bal) => {
                    if bal != row.balance {
                        data_warn!(
                            "migration verification: guild_wallets: (guild={}, user={}) \
                             balance mismatch (expected {}, got {bal})",
                            row.guild_id,
                            row.user_id,
                            row.balance
                        );
                    }
                }
            }
        }

        // aggregate checksum: total balance
        let expected_wallet_sum: u64 = surreal_wallets.iter().map(|r| r.balance).sum();
        let actual_wallet_sum: u64 = wallets.iter().map(|r| r.balance).sum();
        if expected_wallet_sum != actual_wallet_sum {
            data_warn!(
                "migration verification: guild_wallets: balance sum mismatch \
                 (expected {expected_wallet_sum}, got {actual_wallet_sum})"
            );
        } else {
            info!("migration verification: guild_wallets balance sum ok ({actual_wallet_sum})");
        }
    }

    // =========================================================================
    // guild_payouts
    // =========================================================================
    info!("migration verification: checking guild_payouts");
    let mysql_payouts: Option<Vec<GuildPayout>> = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        match guild_payouts::table
            .select(GuildPayout::as_select())
            .load::<GuildPayout>(&mut conn)
            .await
        {
            Ok(rows) => Some(rows),
            Err(e) => {
                data_warn!("migration verification: could not load guild_payouts: {e}");
                None
            }
        }
    };

    if let Some(ref payouts) = mysql_payouts {
        let mysql_map: HashMap<(i64, i64), &GuildPayout> = payouts
            .iter()
            .map(|r| ((r.guild_id, r.user_id), r))
            .collect();

        // spot-check: balance and last_payout per (guild_id, user_id)
        for row in surreal_payouts {
            match mysql_map.get(&(row.guild_id, row.user_id)) {
                None => data_warn!(
                    "migration verification: guild_payouts: (guild={}, user={}) missing in mysql",
                    row.guild_id,
                    row.user_id
                ),
                Some(&m) => {
                    if m.balance != row.balance {
                        data_warn!(
                            "migration verification: guild_payouts: (guild={}, user={}) \
                             balance mismatch (expected {}, got {})",
                            row.guild_id,
                            row.user_id,
                            row.balance,
                            m.balance
                        );
                    }
                    let expected_payout = row.last_payout.naive_utc();
                    if m.last_payout.trunc_subsecs(0) != expected_payout.trunc_subsecs(0) {
                        data_warn!(
                            "migration verification: guild_payouts: (guild={}, user={}) \
                             last_payout mismatch (expected {expected_payout}, got {})",
                            row.guild_id,
                            row.user_id,
                            m.last_payout
                        );
                    }
                }
            }
        }

        // aggregate checksum: total balance
        let expected_payout_sum: u64 = surreal_payouts.iter().map(|r| r.balance).sum();
        let actual_payout_sum: u64 = payouts.iter().map(|r| r.balance).sum();
        if expected_payout_sum != actual_payout_sum {
            data_warn!(
                "migration verification: guild_payouts: balance sum mismatch \
                 (expected {expected_payout_sum}, got {actual_payout_sum})"
            );
        } else {
            info!("migration verification: guild_payouts balance sum ok ({actual_payout_sum})");
        }
    }

    // =========================================================================
    // quotes
    // =========================================================================
    info!("migration verification: checking quotes");
    let mysql_quotes: Option<Vec<Quote>> = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        match quotes::table
            .select(Quote::as_select())
            .order(quotes::sequential_id.asc())
            .load::<Quote>(&mut conn)
            .await
        {
            Ok(rows) => Some(rows),
            Err(e) => {
                data_warn!("migration verification: could not load quotes: {e}");
                None
            }
        }
    };

    if let Some(ref mysql_qs) = mysql_quotes {
        // build map by sequential_id (1-based, matches surreal index + 1)
        let mysql_map: HashMap<i32, &Quote> =
            mysql_qs.iter().map(|r| (r.sequential_id, r)).collect();

        // spot-check: quote text, invoker, stream_category per sequential_id
        for (i, row) in surreal_quotes.iter().enumerate() {
            let seq_id = (i + 1) as i32;
            match mysql_map.get(&seq_id) {
                None => data_warn!(
                    "migration verification: quotes: sequential_id {seq_id} missing in mysql"
                ),
                Some(&m) => {
                    if m.quote != row.quote {
                        data_warn!(
                            "migration verification: quotes: sequential_id {seq_id} \
                             quote text mismatch"
                        );
                    }
                    if m.invoker != row.invoker {
                        data_warn!(
                            "migration verification: quotes: sequential_id {seq_id} \
                             invoker mismatch (expected {:?}, got {:?})",
                            row.invoker,
                            m.invoker
                        );
                    }
                    if m.stream_category != row.stream_category {
                        data_warn!(
                            "migration verification: quotes: sequential_id {seq_id} \
                             stream_category mismatch"
                        );
                    }
                }
            }
        }

        // empty-value checks
        let empty_text = mysql_qs.iter().filter(|r| r.quote.is_empty()).count();
        if empty_text > 0 {
            data_warn!("migration verification: quotes: {empty_text} rows have empty quote text");
        }
        let empty_invoker = mysql_qs.iter().filter(|r| r.invoker.is_empty()).count();
        if empty_invoker > 0 {
            data_warn!("migration verification: quotes: {empty_invoker} rows have empty invoker");
        }

        // referential integrity: every quote must reference default_community_id
        let wrong_community = mysql_qs
            .iter()
            .filter(|r| r.community_id != default_community_id)
            .count();
        if wrong_community > 0 {
            data_warn!(
                "migration verification: quotes: {wrong_community} rows reference unexpected \
                 community_id (expected all to be {default_community_id})"
            );
        } else if !mysql_qs.is_empty() {
            info!(
                "migration verification: quotes referential integrity ok (all {} rows reference \
                 community {default_community_id})",
                mysql_qs.len()
            );
        }
    }

    all_ok
}

/// Performs a one-time migration of all data from SurrealDB into the MySQL
/// database.
///
/// This function is idempotent: if `guild_configs` already has rows, it logs a
/// message and returns immediately without touching anything.
///
/// After migration is verified, call this function's call site can be removed
/// together with this module.
pub async fn migrate_from_surrealdb<C: Connection>(
    pool: &DbPool,
    surreal: &Surreal<C>,
) -> Result<(), MuniBotError> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    // idempotency check: skip if mysql already has data
    //
    // both guild_configs and community_links are checked because a run with
    // zero logging channels would leave guild_configs empty while still
    // creating the default community link -- checking only guild_configs would
    // allow the migration to re-run and fail on the community link unique
    // constraint
    let (config_count, community_count): (i64, i64) = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        let c = guild_configs::table
            .count()
            .get_result(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?;
        let cl = community_links::table
            .count()
            .get_result(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?;
        (c, cl)
    };
    if config_count > 0 || community_count > 0 {
        info!("migration: mysql already has data, skipping migration");
        return Ok(());
    }

    info!("migration: starting SurrealDB -> MySQL migration");

    // --- 1. migrate logging_channel -> guild_configs ---
    let surreal_log_channels: Vec<SurrealLoggingChannel> = surreal
        .query("SELECT * FROM logging_channel")
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} logging channels",
        surreal_log_channels.len()
    );
    // track only the records that were actually inserted; non-numeric ids are
    // skipped with a warning and must not count toward the verification total
    let mut migrated_log_count: i64 = 0;
    for row in &surreal_log_channels {
        let guild_id = match i64::try_from(row.id.key().clone()) {
            Ok(n) => n,
            Err(_) => {
                warn!(
                    "migration: skipping logging_channel with non-numeric id: {:?}",
                    row.id
                );
                continue;
            }
        };
        operations::upsert_guild_config(
            pool,
            GuildConfig {
                guild_id,
                logging_channel: Some(row.channel_id),
            },
        )
        .await
        .map_err(MuniBotError::DbError)?;
        migrated_log_count += 1;
    }

    // --- 2. migrate autodelete_timer -> autodelete_timers ---
    let surreal_timers: Vec<SurrealAutoDeleteTimer> = surreal
        .query("SELECT * FROM autodelete_timer")
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} autodelete timers",
        surreal_timers.len()
    );
    for row in &surreal_timers {
        operations::upsert_autodelete_timer(
            pool,
            AutoDeleteTimerRow {
                channel_id: row.channel_id,
                guild_id: row.guild_id,
                duration_secs: row.duration.as_secs() as i64,
                last_cleaned: row.last_cleaned.naive_utc(),
                last_message_id_cleaned: row.last_message_id_cleaned,
                mode: row.mode.clone(),
            },
        )
        .await
        .map_err(MuniBotError::DbError)?;
    }

    // --- 3. migrate guild_wallet -> guild_wallets ---
    let surreal_wallets: Vec<SurrealGuildWallet> = surreal
        .query("SELECT * FROM guild_wallet")
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} guild wallets",
        surreal_wallets.len()
    );
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in &surreal_wallets {
            diesel::insert_into(guild_wallets::table)
                .values(NewGuildWallet {
                    guild_id: row.guild_id,
                    user_id: row.user_id,
                    balance: row.balance,
                })
                .execute(&mut conn)
                .await
                .map_err(MuniBotError::DbError)?;
        }
    }

    // --- 4. migrate guild_payout -> guild_payouts ---
    let surreal_payouts: Vec<SurrealGuildPayout> = surreal
        .query("SELECT * FROM guild_payout")
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} guild payouts",
        surreal_payouts.len()
    );
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in &surreal_payouts {
            diesel::insert_into(guild_payouts::table)
                .values(NewGuildPayout {
                    guild_id: row.guild_id,
                    user_id: row.user_id,
                    balance: row.balance,
                    last_payout: row.last_payout.naive_utc(),
                })
                .execute(&mut conn)
                .await
                .map_err(MuniBotError::DbError)?;
        }
    }

    // --- 5. create default community_link for existing quotes ---
    let default_community = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        diesel::insert_into(community_links::table)
            .values(NewCommunityLink {
                twitch_streamer_id: Some(DEFAULT_TWITCH_STREAMER_ID.to_owned()),
                discord_guild_id: None,
            })
            .execute(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?;

        community_links::table
            .filter(community_links::twitch_streamer_id.eq(DEFAULT_TWITCH_STREAMER_ID))
            .select(crate::db::models::CommunityLink::as_select())
            .first(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    // --- 6. migrate quote -> quotes ---
    // order by created_at to assign sequential_id in chronological order
    let surreal_quotes: Vec<SurrealQuote> = surreal
        .query("SELECT * FROM quote ORDER BY created_at ASC")
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!("migration: migrating {} quotes", surreal_quotes.len());
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for (i, row) in surreal_quotes.iter().enumerate() {
            diesel::insert_into(quotes::table)
                .values(crate::db::models::NewQuote {
                    community_id: default_community.id,
                    sequential_id: (i + 1) as i32,
                    created_at: row.created_at.naive_utc(),
                    quote: row.quote.clone(),
                    invoker: row.invoker.clone(),
                    stream_category: row.stream_category.clone(),
                    stream_title: row.stream_title.clone(),
                })
                .execute(&mut conn)
                .await
                .map_err(MuniBotError::DbError)?;
        }
    }

    // --- 7. verification ---
    info!("migration: verifying row counts");

    let mysql_log_count = {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        let mut conn = pool.get().await.expect("couldn't get db connection");
        guild_configs::table
            .count()
            .get_result::<i64>(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    let mysql_timer_count = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        autodelete_timers::table
            .count()
            .get_result::<i64>(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    let mysql_wallet_count = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        guild_wallets::table
            .count()
            .get_result::<i64>(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    let mysql_payout_count = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        guild_payouts::table
            .count()
            .get_result::<i64>(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    let mysql_quote_count = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        quotes::table
            .count()
            .get_result::<i64>(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };

    // use migrated_log_count (not raw surreal count) so that skipped
    // non-numeric ids do not cause a spurious count mismatch
    let surreal_log_count = migrated_log_count;
    let surreal_timer_count = surreal_timers.len() as i64;
    let surreal_wallet_count = surreal_wallets.len() as i64;
    let surreal_payout_count = surreal_payouts.len() as i64;
    let surreal_quote_count = surreal_quotes.len() as i64;

    let mut mismatch = false;

    macro_rules! verify_count {
        ($table:expr, $surreal:expr, $mysql:expr) => {
            if $surreal != $mysql {
                log::error!(
                    "migration: count mismatch for {}: surreal={}, mysql={}",
                    $table,
                    $surreal,
                    $mysql
                );
                mismatch = true;
            } else {
                info!("migration: {} ok ({} rows)", $table, $mysql);
            }
        };
    }

    verify_count!("guild_configs", surreal_log_count, mysql_log_count);
    verify_count!("autodelete_timers", surreal_timer_count, mysql_timer_count);
    verify_count!("guild_wallets", surreal_wallet_count, mysql_wallet_count);
    verify_count!("guild_payouts", surreal_payout_count, mysql_payout_count);
    verify_count!("quotes", surreal_quote_count, mysql_quote_count);

    if mismatch {
        return Err(MuniBotError::Other(
            "migration: count verification failed; see error logs above".to_owned(),
        ));
    }

    info!("migration: all counts verified successfully");

    // --- 8. data verification ---
    info!("migration: starting data verification (field values, checksums, nulls, refs)");
    let data_ok = verify_migration_data(
        pool,
        &surreal_log_channels,
        &surreal_timers,
        &surreal_wallets,
        &surreal_payouts,
        &surreal_quotes,
        default_community.id,
    )
    .await;

    if data_ok {
        info!("migration: data verification passed");
    } else {
        warn!("migration: data verification found issues; see warnings above");
    }

    Ok(())
}
