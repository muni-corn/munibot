use chrono::{DateTime, Local, Utc};
use log::{info, warn};
use serde::Deserialize;
use surrealdb::{Surreal, engine::remote::ws};

use crate::{
    MuniBotError,
    db::{
        DbPool,
        models::{
            AutoDeleteTimerRow, GuildConfig, NewCommunityLink, NewGuildPayout, NewGuildWallet,
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

/// Performs a one-time migration of all data from SurrealDB into the MySQL
/// database.
///
/// This function is idempotent: if `guild_configs` already has rows, it logs a
/// message and returns immediately without touching anything.
///
/// After migration is verified, call this function's call site can be removed
/// together with this module.
pub async fn migrate_from_surrealdb(
    pool: &DbPool,
    surreal: &Surreal<ws::Client>,
) -> Result<(), MuniBotError> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    // idempotency check: skip if mysql already has data
    let count: i64 = {
        let mut conn = pool.get().await.expect("couldn't get db connection");
        guild_configs::table
            .count()
            .get_result(&mut conn)
            .await
            .map_err(MuniBotError::DbError)?
    };
    if count > 0 {
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

    let surreal_log_count = surreal_log_channels.len() as i64;
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
    Ok(())
}
