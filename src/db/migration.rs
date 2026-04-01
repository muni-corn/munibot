use chrono::{DateTime, SubsecRound, Utc};
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
            autodelete_timers::{self},
            community_links, guild_configs, guild_payouts, guild_wallets,
            quotes::{self},
        },
    },
    passing::Passing,
};

/// The Twitch streamer ID used for the default community that existing quotes
/// will be migrated into.
const DEFAULT_TWITCH_STREAMER_ID: &str = "590712444";

// SurrealDB record shapes (read-only, used only during migration)

#[derive(Debug, Deserialize)]
pub struct SurrealLoggingChannel {
    // surrealdb record id key is the guild id
    pub id: surrealdb::RecordId,
    pub channel_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SurrealAutoDeleteTimer {
    pub guild_id: i64,
    pub channel_id: i64,
    #[serde(with = "humantime_serde")]
    pub duration: std::time::Duration,
    pub last_cleaned: DateTime<Utc>,
    #[serde(default)]
    pub last_message_id_cleaned: i64,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct SurrealGuildWallet {
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
}

#[derive(Debug, Deserialize)]
pub struct SurrealGuildPayout {
    pub guild_id: i64,
    pub user_id: i64,
    pub balance: u64,
    pub last_payout: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SurrealQuote {
    pub created_at: DateTime<Utc>,
    pub quote: String,
    pub invoker: String,
    pub stream_category: String,
    pub stream_title: String,
}

/// Data that was actually, successfully migrated from SurrealDB into MySQL
/// during a single run of `migrate_from_surrealdb`.
///
/// On an idempotent re-run where all data already exists, every vec will be
/// empty.
pub struct MigratedData {
    pub surreal_log_channels: Vec<SurrealLoggingChannel>,
    pub surreal_timers: Vec<SurrealAutoDeleteTimer>,
    pub surreal_wallets: Vec<SurrealGuildWallet>,
    pub surreal_payouts: Vec<SurrealGuildPayout>,
    pub surreal_quotes: Vec<SurrealQuote>,
    pub default_community_id: i64,
}

impl MigratedData {
    fn is_empty(&self) -> bool {
        self.surreal_log_channels.is_empty()
            && self.surreal_timers.is_empty()
            && self.surreal_wallets.is_empty()
            && self.surreal_payouts.is_empty()
            && self.surreal_quotes.is_empty()
    }
}

/// Verifies data completeness and accuracy after migration.
///
/// Performs three categories of checks against the rows that were actually
/// inserted this run:
///
/// 1. **Spot-check field values:** loads MySQL rows and compares key fields
///    against the in-memory SurrealDB source data.
/// 2. **Null/empty field validation:** warns on rows with empty strings or zero
///    values in fields that should always be populated.
/// 3. **Referential integrity:** verifies every migrated quote references the
///    correct `community_id`.
///
/// All failures are logged as warnings; this function never returns an error.
/// Returns `true` if every check passed, `false` if any mismatch was found.
async fn verify_migration_data(
    pool: &DbPool,
    MigratedData {
        surreal_log_channels,
        surreal_timers,
        surreal_wallets,
        surreal_payouts,
        surreal_quotes,
        default_community_id,
    }: &MigratedData,
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
        if zero_dur == 0 && empty_mode == 0 {
            info!("migration verification: autodelete_timers field values ok");
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
        let mut all_wallets_ok = true;
        for row in surreal_wallets {
            match mysql_map.get(&(row.guild_id, row.user_id)) {
                None => {
                    data_warn!(
                        "migration verification: guild_wallets: (guild={}, user={}) missing \
                         in mysql",
                        row.guild_id,
                        row.user_id
                    );
                    all_wallets_ok = false;
                }
                Some(&bal) => {
                    if bal != row.balance {
                        data_warn!(
                            "migration verification: guild_wallets: (guild={}, user={}) \
                             balance mismatch (expected {}, got {bal})",
                            row.guild_id,
                            row.user_id,
                            row.balance
                        );
                        all_wallets_ok = false;
                    }
                }
            }
        }
        if all_wallets_ok {
            info!("migration verification: guild_wallets field values ok");
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
        let mut all_payouts_ok = true;
        for row in surreal_payouts {
            match mysql_map.get(&(row.guild_id, row.user_id)) {
                None => {
                    data_warn!(
                        "migration verification: guild_payouts: (guild={}, user={}) missing \
                         in mysql",
                        row.guild_id,
                        row.user_id
                    );
                    all_payouts_ok = false;
                }
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
                        all_payouts_ok = false;
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
                        all_payouts_ok = false;
                    }
                }
            }
        }
        if all_payouts_ok {
            info!("migration verification: guild_payouts field values ok");
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

        // referential integrity: every migrated quote must reference
        // default_community_id
        let wrong_community = mysql_qs
            .iter()
            .filter(|r| r.community_id != *default_community_id)
            .count();
        if wrong_community > 0 {
            data_warn!(
                "migration verification: quotes: {wrong_community} rows reference unexpected \
                 community_id (expected all to be {default_community_id})"
            );
        } else if !surreal_quotes.is_empty() {
            info!(
                "migration verification: quotes referential integrity ok (all {} migrated rows \
                 reference community {default_community_id})",
                surreal_quotes.len()
            );
        }
    }

    all_ok
}

/// Performs a one-time migration of all data from SurrealDB into the MySQL
/// database.
///
/// This function is idempotent: existing rows are detected via
/// `INSERT OR IGNORE` and skipped. Only rows actually inserted this run are
/// tracked in the returned [`MigratedData`].
///
/// After migration is verified, this function's call site can be removed
/// together with this module.
pub async fn migrate_from_surrealdb<C: Connection>(
    pool: &DbPool,
    surreal: &Surreal<C>,
) -> Result<MigratedData, MuniBotError> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    info!("migration: starting SurrealDB -> MySQL migration");

    // --- 1. migrate logging_channel -> guild_configs ---
    let surreal_log_channels: Vec<SurrealLoggingChannel> = surreal
        .query("SELECT id, <int>channel_id AS channel_id FROM logging_channel")
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
    let mut migrated_surreal_log_channels: Vec<SurrealLoggingChannel> = Vec::new();
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in surreal_log_channels {
            let guild_id = match i64::try_from(row.id.key().clone()) {
                Ok(n) => n,
                Err(e) => {
                    warn!(
                        "migration: skipping logging_channel with non-numeric id {:?}: {e}",
                        row.id
                    );
                    continue;
                }
            };

            let affected = diesel::insert_or_ignore_into(guild_configs::table)
                .values(&GuildConfig {
                    guild_id,
                    logging_channel: Some(row.channel_id),
                })
                .execute(&mut conn)
                .await;

            match affected {
                Ok(0) => info!("guild config for {guild_id} already exists; skipping"),
                Ok(1) => {
                    info!("migrated guild config for {guild_id}");
                    migrated_surreal_log_channels.push(row);
                }
                Ok(n) => {
                    warn!("migrated guild config for {guild_id}, but {n} rows were affected");
                    migrated_surreal_log_channels.push(row);
                }
                Err(e) => warn!("couldn't migrate guild config for {guild_id}: {e}"),
            }
        }
    }

    // --- 2. migrate autodelete_timer -> autodelete_timers ---
    let surreal_timers: Vec<SurrealAutoDeleteTimer> = surreal
        .query(
            "SELECT \
             <int>channel_id AS channel_id, \
             <int>guild_id AS guild_id, \
             duration, \
             <datetime>last_cleaned AS last_cleaned, \
             <int>(last_message_id_cleaned ?? 0) AS last_message_id_cleaned, \
             mode \
             FROM autodelete_timer",
        )
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} autodelete timers",
        surreal_timers.len()
    );
    let mut migrated_surreal_timers: Vec<SurrealAutoDeleteTimer> = Vec::new();
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in surreal_timers {
            let affected = diesel::insert_or_ignore_into(autodelete_timers::table)
                .values(&AutoDeleteTimerRow {
                    channel_id: row.channel_id,
                    guild_id: row.guild_id,
                    duration_secs: row.duration.as_secs() as i64,
                    last_cleaned: row.last_cleaned.naive_utc(),
                    last_message_id_cleaned: row.last_message_id_cleaned,
                    mode: row.mode.clone(),
                })
                .execute(&mut conn)
                .await;

            match affected {
                Ok(0) => info!(
                    "autodelete timer for channel {} already exists; skipping",
                    row.channel_id
                ),
                Ok(1) => {
                    info!("migrated autodelete timer for channel {}", row.channel_id);
                    migrated_surreal_timers.push(row);
                }
                Ok(n) => {
                    warn!(
                        "migrated autodelete timer for channel {}, but {n} rows were affected",
                        row.channel_id
                    );
                    migrated_surreal_timers.push(row);
                }
                Err(e) => warn!(
                    "couldn't migrate autodelete timer for channel {}: {e}",
                    row.channel_id
                ),
            }
        }
    }

    // --- 3. migrate guild_wallet -> guild_wallets ---
    let surreal_wallets: Vec<SurrealGuildWallet> = surreal
        .query(
            "SELECT <int>guild_id AS guild_id, <int>user_id AS user_id, balance \
             FROM guild_wallet",
        )
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} guild wallets",
        surreal_wallets.len()
    );
    let mut migrated_surreal_wallets: Vec<SurrealGuildWallet> = Vec::new();
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in surreal_wallets {
            let affected = diesel::insert_or_ignore_into(guild_wallets::table)
                .values(NewGuildWallet {
                    guild_id: row.guild_id,
                    user_id: row.user_id,
                    balance: row.balance,
                })
                .execute(&mut conn)
                .await;

            match affected {
                Ok(0) => info!(
                    "guild wallet for (guild={}, user={}) already exists; skipping",
                    row.guild_id, row.user_id
                ),
                Ok(1) => {
                    info!(
                        "migrated guild wallet for (guild={}, user={})",
                        row.guild_id, row.user_id
                    );
                    migrated_surreal_wallets.push(row);
                }
                Ok(n) => {
                    warn!(
                        "migrated guild wallet for (guild={}, user={}), but {n} rows were affected",
                        row.guild_id, row.user_id
                    );
                    migrated_surreal_wallets.push(row);
                }
                Err(e) => warn!(
                    "couldn't migrate guild wallet for (guild={}, user={}): {e}",
                    row.guild_id, row.user_id
                ),
            }
        }
    }

    // --- 4. migrate guild_payout -> guild_payouts ---
    let surreal_payouts: Vec<SurrealGuildPayout> = surreal
        .query(
            "SELECT \
             <int>guild_id AS guild_id, \
             <int>user_id AS user_id, \
             balance, \
             <datetime>last_payout AS last_payout \
             FROM guild_payout",
        )
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!(
        "migration: migrating {} guild payouts",
        surreal_payouts.len()
    );
    let mut migrated_surreal_payouts: Vec<SurrealGuildPayout> = Vec::new();
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for row in surreal_payouts {
            let affected = diesel::insert_or_ignore_into(guild_payouts::table)
                .values(NewGuildPayout {
                    guild_id: row.guild_id,
                    user_id: row.user_id,
                    balance: row.balance,
                    last_payout: row.last_payout.naive_utc(),
                })
                .execute(&mut conn)
                .await;

            match affected {
                Ok(0) => info!(
                    "guild payout for (guild={}, user={}) already exists; skipping",
                    row.guild_id, row.user_id
                ),
                Ok(1) => {
                    info!(
                        "migrated guild payout for (guild={}, user={})",
                        row.guild_id, row.user_id
                    );
                    migrated_surreal_payouts.push(row);
                }
                Ok(n) => {
                    warn!(
                        "migrated guild payout for (guild={}, user={}), but {n} rows were affected",
                        row.guild_id, row.user_id
                    );
                    migrated_surreal_payouts.push(row);
                }
                Err(e) => warn!(
                    "couldn't migrate guild payout for (guild={}, user={}): {e}",
                    row.guild_id, row.user_id
                ),
            }
        }
    }

    // --- 5. create default community_link for existing quotes ---
    let default_community = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        diesel::insert_or_ignore_into(community_links::table)
            .values(NewCommunityLink {
                twitch_streamer_id: Some(DEFAULT_TWITCH_STREAMER_ID.to_owned()),
                discord_guild_id: None,
            })
            .execute(&mut conn)
            .await
            .pass();

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
        .query(
            "SELECT \
             <datetime>created_at AS created_at, \
             quote, invoker, stream_category, stream_title \
             FROM quote \
             ORDER BY created_at ASC",
        )
        .await
        .map_err(|e| MuniBotError::Other(format!("surreal query failed: {e}")))?
        .take(0)
        .map_err(|e| MuniBotError::Other(format!("surreal take failed: {e}")))?;

    info!("migration: migrating {} quotes", surreal_quotes.len());
    let mut migrated_surreal_quotes: Vec<SurrealQuote> = Vec::new();
    {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        for (i, row) in surreal_quotes.into_iter().enumerate() {
            // check by content so that a partial re-run doesn't assign a
            // different sequential_id to an already-migrated quote
            let existing = match operations::get_quote_by_content(
                pool,
                default_community.id,
                &row.quote,
            )
            .await
            {
                Ok(q) => q,
                Err(e) => {
                    log::error!("couldn't check existing quote {}: {e}; skipping", i + 1);
                    continue;
                }
            };

            if existing.is_none() {
                let affected = diesel::insert_or_ignore_into(quotes::table)
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
                    .await;

                match affected {
                    Ok(0) => info!("quote {} already exists; skipping", i + 1),
                    Ok(1) => {
                        info!(
                            "migrated quote {} into community {}: \"{}\"",
                            i + 1,
                            default_community.id,
                            &row.quote
                        );
                        migrated_surreal_quotes.push(row);
                    }
                    Ok(n) => {
                        info!(
                            "migrated quote {} into community {}: \"{}\", but {n} rows were affected",
                            i + 1,
                            default_community.id,
                            &row.quote
                        );
                        migrated_surreal_quotes.push(row);
                    }
                    Err(e) => warn!("couldn't migrate quote {}: {e}", i + 1),
                }
            } else {
                info!("quote {} already exists; skipping", i + 1);
            }
        }
    }

    // --- 7. data verification ---
    // only verify if something was actually migrated this run; on an idempotent
    // re-run the migrated vecs are empty and there is nothing new to validate

    let migrated_data = MigratedData {
        surreal_log_channels: migrated_surreal_log_channels,
        surreal_timers: migrated_surreal_timers,
        surreal_wallets: migrated_surreal_wallets,
        surreal_payouts: migrated_surreal_payouts,
        surreal_quotes: migrated_surreal_quotes,
        default_community_id: default_community.id,
    };

    let anything_migrated = !migrated_data.is_empty();

    if anything_migrated {
        info!("migration: starting data verification (field values, nulls, refs)");
        let data_ok = verify_migration_data(pool, &migrated_data).await;

        if data_ok {
            info!("migration: data verification passed");
        } else {
            warn!("migration: data verification found issues; see warnings above");
        }
    } else {
        info!("migration: nothing new was migrated; skipping data verification");
    }

    Ok(migrated_data)
}
