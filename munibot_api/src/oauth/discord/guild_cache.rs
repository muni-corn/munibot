//! A short-lived, per-user cache of `get_current_user_guilds` results.
//!
//! `GET /users/@me/guilds` is rate limited to roughly one request per
//! second per user token, but munibot calls it multiple times for a single
//! page load (the dashboard's guild list, plus a guild-admin check per
//! settings server function). Without this cache those calls land within
//! milliseconds of each other and reliably 429.
//!
//! `moka`'s `try_get_with` gives this two properties a plain
//! `HashMap<UserId, (Instant, Vec<DiscordGuild>)>` wouldn't, for free:
//! bounded size with lru-ish eviction, and single-flight coalescing --
//! concurrent callers that all miss the cache for the same user share one
//! in-flight request to discord instead of each firing their own.
use std::{sync::LazyLock, time::Duration};

use moka::future::Cache;

use super::{DiscordGuild, DiscordOAuthError};

/// How long a user's guild list is cached for. This doubles as an
/// authorization staleness window: a user demoted from a guild (or who
/// leaves it) keeps whatever access `is_administered_by_user` granted them
/// for up to this long. Settings writes still re-validate against the
/// guild's real state server-side (e.g. a channel id has to actually exist
/// in the guild), so the blast radius of that staleness is small.
const TIME_TO_LIVE: Duration = Duration::from_secs(60);

/// A generous ceiling: not a real limit on concurrent users, just enough to
/// keep an idle, misbehaving client from growing this unboundedly.
const MAX_CAPACITY: u64 = 10_000;

type GuildList = std::sync::Arc<Vec<DiscordGuild>>;

static CACHE: LazyLock<Cache<i64, GuildList>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(MAX_CAPACITY)
        .time_to_live(TIME_TO_LIVE)
        .build()
});

/// Returns the guilds `user_id`'s linked discord account (identified by
/// `access_token`) is a member of, serving a cached copy if one was fetched
/// within the last `TIME_TO_LIVE`.
///
/// Concurrent calls for the same `user_id` that all miss the cache share a
/// single request to discord.
pub async fn guilds_for_user(
    user_id: i64,
    access_token: &str,
) -> Result<GuildList, std::sync::Arc<DiscordOAuthError>> {
    CACHE
        .try_get_with(user_id, async {
            super::get_current_user_guilds(access_token)
                .await
                .map(std::sync::Arc::new)
        })
        .await
}

/// Drops any cached guild list for `user_id`. Called on sign-in and
/// sign-out, so neither a fresh session nor a signed-out one ever reads a
/// list fetched under a previous session's token.
pub async fn invalidate(user_id: i64) {
    CACHE.invalidate(&user_id).await;
}
