use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{
        AutoDeleteTimerRow, CommunityLink, GuildConfig, GuildPayout, GuildWallet, NewCommunityLink,
        NewGuildPayout, NewGuildWallet, NewQuote, Quote, UpdateAutoDeleteTimer,
    },
    schema::{
        autodelete_timers, community_links, guild_configs, guild_payouts, guild_wallets, quotes,
    },
};

// guild_configs

/// Inserts or updates a guild config row, returning the saved record.
///
/// Uses MySQL's `REPLACE INTO` which deletes the old row and inserts the new
/// one when a duplicate primary key is found.
pub async fn upsert_guild_config(pool: &DbPool, config: GuildConfig) -> QueryResult<GuildConfig> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::replace_into(guild_configs::table)
        .values(&config)
        .execute(&mut conn)
        .await?;
    guild_configs::table
        .find(config.guild_id)
        .select(GuildConfig::as_select())
        .first(&mut conn)
        .await
}

/// Retrieves a guild config by guild ID, returning `None` if not found.
pub async fn get_guild_config(pool: &DbPool, guild_id: i64) -> QueryResult<Option<GuildConfig>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    guild_configs::table
        .find(guild_id)
        .select(GuildConfig::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Deletes a guild config row by guild ID.
pub async fn delete_guild_config(pool: &DbPool, guild_id: i64) -> QueryResult<usize> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(guild_configs::table.find(guild_id))
        .execute(&mut conn)
        .await
}

// autodelete_timers

/// Returns all autodelete timer rows.
pub async fn get_all_autodelete_timers(pool: &DbPool) -> QueryResult<Vec<AutoDeleteTimerRow>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    autodelete_timers::table
        .select(AutoDeleteTimerRow::as_select())
        .load(&mut conn)
        .await
}

/// Inserts or replaces an autodelete timer row.
pub async fn upsert_autodelete_timer(
    pool: &DbPool,
    row: AutoDeleteTimerRow,
) -> QueryResult<AutoDeleteTimerRow> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::replace_into(autodelete_timers::table)
        .values(&row)
        .execute(&mut conn)
        .await?;
    autodelete_timers::table
        .find(row.channel_id)
        .select(AutoDeleteTimerRow::as_select())
        .first(&mut conn)
        .await
}

/// Deletes an autodelete timer row by channel ID.
pub async fn delete_autodelete_timer(pool: &DbPool, channel_id: i64) -> QueryResult<usize> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::delete(autodelete_timers::table.find(channel_id))
        .execute(&mut conn)
        .await
}

/// Updates only `last_cleaned` and `last_message_id_cleaned` for a timer row.
pub async fn update_autodelete_last_cleaned(
    pool: &DbPool,
    channel_id: i64,
    last_cleaned: NaiveDateTime,
    last_message_id_cleaned: i64,
) -> QueryResult<usize> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(autodelete_timers::table.find(channel_id))
        .set(UpdateAutoDeleteTimer {
            duration_secs: None,
            last_cleaned: Some(last_cleaned),
            last_message_id_cleaned: Some(last_message_id_cleaned),
            mode: None,
        })
        .execute(&mut conn)
        .await
}

// guild_wallets

/// Returns an existing wallet for the given guild and user, or creates one with
/// a zero balance.
pub async fn get_or_create_wallet(
    pool: &DbPool,
    guild_id: i64,
    user_id: i64,
) -> QueryResult<GuildWallet> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let existing = guild_wallets::table
        .filter(guild_wallets::guild_id.eq(guild_id))
        .filter(guild_wallets::user_id.eq(user_id))
        .select(GuildWallet::as_select())
        .first(&mut conn)
        .await
        .optional()?;

    if let Some(wallet) = existing {
        return Ok(wallet);
    }

    diesel::insert_into(guild_wallets::table)
        .values(NewGuildWallet {
            guild_id,
            user_id,
            balance: 0,
        })
        .execute(&mut conn)
        .await?;

    guild_wallets::table
        .filter(guild_wallets::guild_id.eq(guild_id))
        .filter(guild_wallets::user_id.eq(user_id))
        .select(GuildWallet::as_select())
        .first(&mut conn)
        .await
}

/// Overwrites the balance for a wallet row identified by its primary key.
pub async fn update_wallet(pool: &DbPool, id: i64, new_balance: u64) -> QueryResult<usize> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(guild_wallets::table.find(id))
        .set(guild_wallets::balance.eq(new_balance))
        .execute(&mut conn)
        .await
}

/// Adds `amount` to the wallet balance, returning the updated row.
pub async fn deposit_to_wallet(pool: &DbPool, id: i64, amount: u64) -> QueryResult<GuildWallet> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(guild_wallets::table.find(id))
        .set(guild_wallets::balance.eq(guild_wallets::balance + amount))
        .execute(&mut conn)
        .await?;
    guild_wallets::table
        .find(id)
        .select(GuildWallet::as_select())
        .first(&mut conn)
        .await
}

/// Subtracts `amount` from the wallet balance. The caller must ensure there are
/// sufficient funds; no checked subtraction is performed at the SQL layer.
pub async fn spend_from_wallet(pool: &DbPool, id: i64, amount: u64) -> QueryResult<GuildWallet> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(guild_wallets::table.find(id))
        .set(guild_wallets::balance.eq(guild_wallets::balance - amount))
        .execute(&mut conn)
        .await?;
    guild_wallets::table
        .find(id)
        .select(GuildWallet::as_select())
        .first(&mut conn)
        .await
}

// guild_payouts

/// Returns an existing payout entry for the given guild and user, or creates
/// one with a zero balance and `last_payout` set to `initial_last_payout`.
pub async fn get_or_create_payout(
    pool: &DbPool,
    guild_id: i64,
    user_id: i64,
    initial_last_payout: NaiveDateTime,
) -> QueryResult<GuildPayout> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let existing = guild_payouts::table
        .filter(guild_payouts::guild_id.eq(guild_id))
        .filter(guild_payouts::user_id.eq(user_id))
        .select(GuildPayout::as_select())
        .first(&mut conn)
        .await
        .optional()?;

    if let Some(payout) = existing {
        return Ok(payout);
    }

    diesel::insert_into(guild_payouts::table)
        .values(NewGuildPayout {
            guild_id,
            user_id,
            balance: 0,
            last_payout: initial_last_payout,
        })
        .execute(&mut conn)
        .await?;

    guild_payouts::table
        .filter(guild_payouts::guild_id.eq(guild_id))
        .filter(guild_payouts::user_id.eq(user_id))
        .select(GuildPayout::as_select())
        .first(&mut conn)
        .await
}

/// Overwrites the balance for a payout row identified by its primary key.
pub async fn update_payout(pool: &DbPool, id: i64, new_balance: u64) -> QueryResult<usize> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(guild_payouts::table.find(id))
        .set(guild_payouts::balance.eq(new_balance))
        .execute(&mut conn)
        .await
}

/// Drains the payout balance to zero, records `last_payout`, and returns the
/// updated row.
pub async fn claim_payout(
    pool: &DbPool,
    id: i64,
    claimed_at: NaiveDateTime,
) -> QueryResult<GuildPayout> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(guild_payouts::table.find(id))
        .set((
            guild_payouts::balance.eq(0u64),
            guild_payouts::last_payout.eq(claimed_at),
        ))
        .execute(&mut conn)
        .await?;
    guild_payouts::table
        .find(id)
        .select(GuildPayout::as_select())
        .first(&mut conn)
        .await
}

// community_links

/// Returns an existing community link by Twitch streamer ID, creating one if
/// none exists.
pub async fn get_or_create_community_link_by_twitch_id(
    pool: &DbPool,
    twitch_streamer_id: &str,
) -> QueryResult<CommunityLink> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let existing = community_links::table
        .filter(community_links::twitch_streamer_id.eq(twitch_streamer_id))
        .select(CommunityLink::as_select())
        .first(&mut conn)
        .await
        .optional()?;

    if let Some(link) = existing {
        return Ok(link);
    }

    diesel::insert_into(community_links::table)
        .values(NewCommunityLink {
            twitch_streamer_id: Some(twitch_streamer_id.to_owned()),
            discord_guild_id: None,
        })
        .execute(&mut conn)
        .await?;

    community_links::table
        .filter(community_links::twitch_streamer_id.eq(twitch_streamer_id))
        .select(CommunityLink::as_select())
        .first(&mut conn)
        .await
}

/// Returns the community link for a given Twitch streamer ID, if any.
pub async fn get_community_by_twitch_id(
    pool: &DbPool,
    twitch_streamer_id: &str,
) -> QueryResult<Option<CommunityLink>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    community_links::table
        .filter(community_links::twitch_streamer_id.eq(twitch_streamer_id))
        .select(CommunityLink::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Returns the community link for a given Discord guild ID, if any.
pub async fn get_community_by_guild_id(
    pool: &DbPool,
    discord_guild_id: i64,
) -> QueryResult<Option<CommunityLink>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    community_links::table
        .filter(community_links::discord_guild_id.eq(discord_guild_id))
        .select(CommunityLink::as_select())
        .first(&mut conn)
        .await
        .optional()
}

// quotes

/// Inserts a new quote. The `sequential_id` is computed as the next available
/// number for the given community (max + 1, starting at 1 if none exist).
pub async fn add_quote(
    pool: &DbPool,
    community_id: i64,
    created_at: NaiveDateTime,
    quote: String,
    invoker: String,
    stream_category: String,
    stream_title: String,
) -> QueryResult<Quote> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    let next_id: i64 = quotes::table
        .filter(quotes::community_id.eq(community_id))
        .select(diesel::dsl::max(quotes::sequential_id))
        .first::<Option<i32>>(&mut conn)
        .await?
        .map(|max| max as i64 + 1)
        .unwrap_or(1);

    let new_quote = NewQuote {
        community_id,
        sequential_id: next_id as i32,
        created_at,
        quote,
        invoker,
        stream_category,
        stream_title,
    };

    diesel::insert_into(quotes::table)
        .values(&new_quote)
        .execute(&mut conn)
        .await?;

    quotes::table
        .filter(quotes::community_id.eq(community_id))
        .filter(quotes::sequential_id.eq(next_id as i32))
        .select(Quote::as_select())
        .first(&mut conn)
        .await
}

/// Returns a quote by its per-community sequential number, if found.
pub async fn get_quote_by_number(
    pool: &DbPool,
    community_id: i64,
    sequential_id: i32,
) -> QueryResult<Option<Quote>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    quotes::table
        .filter(quotes::community_id.eq(community_id))
        .filter(quotes::sequential_id.eq(sequential_id))
        .select(Quote::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Returns a quote by its content, if found.
pub async fn get_quote_by_content(
    pool: &DbPool,
    community_id: i64,
    content: &str,
) -> QueryResult<Option<Quote>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    quotes::table
        .filter(quotes::community_id.eq(community_id))
        .filter(quotes::quote.like(content))
        .select(Quote::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Returns a random quote for the given community, if any exist.
pub async fn get_random_quote(pool: &DbPool, community_id: i64) -> QueryResult<Option<Quote>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");

    // pick a random sequential_id from all existing ones for this community,
    // then fetch the matching row
    let ids: Vec<i32> = quotes::table
        .filter(quotes::community_id.eq(community_id))
        .select(quotes::sequential_id)
        .load(&mut conn)
        .await?;

    if ids.is_empty() {
        return Ok(None);
    }

    use rand::seq::SliceRandom;
    let chosen = *ids.choose(&mut rand::thread_rng()).unwrap();

    quotes::table
        .filter(quotes::community_id.eq(community_id))
        .filter(quotes::sequential_id.eq(chosen))
        .select(Quote::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Returns the total number of quotes for a given community.
pub async fn count_quotes(pool: &DbPool, community_id: i64) -> QueryResult<i64> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    quotes::table
        .filter(quotes::community_id.eq(community_id))
        .count()
        .get_result(&mut conn)
        .await
}
