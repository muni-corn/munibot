# Notes: discord rate limiting on `GET /users/@me/guilds`

`GET /users/@me/guilds` is limited to roughly one request per second per
user access token (confirmed by discord staff in
[discord-api-docs#670](https://github.com/discord/discord-api-docs/issues/670)).
Before the changes described here, a single dashboard page load could fire
that request two or three times within milliseconds -- once from the
dashboard layout's `get_guilds`, and once per settings server function that
called `require_guild_admin` -- which reliably tripped it.

## What actually fixes it

The rate limiting fix is the cache, not the retry logic. `retry_after` on
this route has been observed asking for over ten minutes at a time (see the
stack overflow thread linked from the same github issue); retrying a burst
of simultaneous requests doesn't make them not be a burst, it just delays
when they all land at once.

`munibot_api/src/oauth/discord/guild_cache.rs` caches a user's guild list
for 60 seconds, keyed by munibot user id. The part that matters for the
burst specifically is that it's backed by moka's `try_get_with`: concurrent
callers that all miss the cache for the same user share one in-flight
request to discord, rather than each firing their own. That's what
collapses a page load's several near-simultaneous calls into one.

The retry-with-backoff in `oauth/discord/rate_limit.rs` is a second,
independent layer on top -- it protects a cold cache (right after the
process starts, or 60 seconds after the last fetch) and the bot-token calls
in `oauth/discord/bot.rs`, which aren't cached at all.

## The cache ttl is also an authorization staleness window

`require_guild_admin` (`munibot_api/src/auth/guild.rs`) is the only thing in
munibot that checks whether a user is allowed to manage a guild's settings,
and it reads from this cache. A user removed from a guild, or demoted below
`MANAGE_GUILD`, keeps whatever dashboard access that granted them for up to
60 seconds.

This is judged an acceptable tradeoff for now because the only write path
gated by it (`set_guild_logging_settings`) re-validates its input against
the guild's real state server-side (the channel id has to actually exist in
that guild, checked via the bot's own token) -- so the blast radius of the
staleness is "can view/change logging settings for up to a minute after
losing access", not "can do anything discord-side munibot can't already
undo".

If a tighter window is ever needed, the ttl lives in
`guild_cache::TIME_TO_LIVE`.

## Why the cache is in-process, not redis

Redis is already connected (it backs sessions, see `docs/gui.md`), but this
cache is a `LazyLock<moka::future::Cache<..>>` static instead. If munibot
ever runs more than one instance behind a load balancer, each instance gets
its own cache -- N instances effectively means N times the calls to
discord, since a user's requests aren't guaranteed to land on the same
instance twice in a row.

This was accepted for now to avoid adding a new `Extension` layer threaded
through every settings server function's `#[server(...)]` attribute (there's
no `AppState` in this codebase to hang it off instead -- state is
`axum::Extension` layers, see `munibot_gui/src/server.rs`). If munibot
becomes multi-instance, moving `GuildList` storage behind redis (serializing
`Vec<DiscordGuild>`) is the fix; the `guild_cache` module boundary is
intentionally the only place that would need to change.
