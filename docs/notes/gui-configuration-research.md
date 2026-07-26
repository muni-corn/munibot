# Notes: configuration ui research (from the abandoned `gui` provider system)

The `gui` branch spent 41 commits (~7,400 lines) building a generic
`ConfigurationProvider` framework -- a `munibot_config` crate with a
`#[derive(Configurable)]` proc macro, a flat path/value/patch model, and a
provider registry -- before writing a single form or server function. It was
abandoned: the framework cost more than the three settings it was meant to
configure (`guild_configs.logging_channel` plus autodelete's duration/mode),
and it deferred every genuinely uncertain part of the problem (Dioxus form
patterns, per-guild authorization, cache invalidation, getting a channel list
at all) until after the framework was built against an imagined shape of all
of those.

What follows is the research from that branch's design doc that's still true
and still useful, without the framework it was built to justify. See
`git show gui:docs/plans/gui-configuration.md` for the full original if the
proc-macro approach is ever worth revisiting at a much larger provider count.

## Existing settings surfaces (as of the fork point)

| Scope                 | Storage             | Read                                                  | Write                                      |
| --------------------- | ------------------- | ----------------------------------------------------- | ------------------------------------------ |
| Guild logging channel | `guild_configs`     | `logging.rs:724-735`, no cache, fresh query per event | `admin.rs:65-70`, delete at `admin.rs:108` |
| Channel autodelete    | `autodelete_timers` | boot-load into a `HashMap`, `autodelete.rs:31-53`     | `autodelete.rs:55-107`, `:111-135`         |
| Global bot config     | TOML file           | `Config::read_or_write_default_from`                  | none -- only writes the default            |

All guild settings today are Discord-slash-command only, gated on
`required_permissions = "MANAGE_GUILD"` (`admin.rs:28`). That gate is the
precedent a GUI equivalent must mirror.

## Storage bugs found by this research (fix before adding columns)

- `upsert_guild_config` (`operations.rs:29`) uses MySQL `REPLACE INTO`, which
  **deletes the row and reinserts it**, nulling every column not present in
  the write. Adding a second guild setting alongside `logging_channel` would
  silently erase the first on every save. Use
  `on_conflict(...).do_update().set(...)` instead (`GuildConfig` already
  derives `AsChangeset`).
- `/admin stop-logging` (`admin.rs:107`) calls `delete_guild_config`, which
  deletes the **whole row** -- not just the logging column. Any future
  per-guild setting sharing that row would be wiped out too. It should
  update `logging_channel` to `NULL` instead.
- `AutoDeleteHandler` loads every timer into a `HashMap` once at boot
  (`autodelete.rs:31-53`) and never re-reads. A GUI (or any external) write
  straight to `autodelete_timers` would be invisible to the running bot
  until restart. Any settings surface touching autodelete-style cached state
  needs an invalidation hook, not just a database write.
- `set_autodelete` (`autodelete.rs:62-73`) always writes
  `last_cleaned: epoch` and `last_message_id_cleaned: 1` alongside the
  duration/mode. Combined with `REPLACE INTO`, **editing an existing timer's
  duration resets its sweep cursor to the beginning of time**, forcing a
  full re-scan. Splitting settings columns from sweep-state columns (or at
  minimum, not re-writing sweep state on a settings-only change) fixes this.

## Authorization gap

`HasPermission::has` (`munibot_api/src/auth/server.rs:63-70`) always returns
`false` -- there is no permission system. `DiscordGuild::is_administered_by_user`
(`oauth/discord.rs`) is currently used only as a **display filter** in
`get_guilds`, never as an authorization check. No server function today
verifies that the calling user actually administers the guild whose settings
it's about to read or write. Any settings server function needs its own
`require_guild_admin`-style check: linked account -> `get_current_user_guilds`
-> `is_administered_by_user`, done fresh (or short-TTL cached) per request,
since there's no local cache of guild membership to check against instead.

## Getting a channel list: the bot token, not the user's oauth token

The OAuth scope granted at sign-in is `identify guilds`
(`oauth/discord.rs:11`), which does **not** permit
`GET /guilds/{id}/channels` -- listing channels needs the bot's own token.

The `gui` branch's plan treated this as requiring shared `Arc<OnceCell<GlobalAccess>>`
plumbing from the bot's `on_ready` into the axum server, so a server function
could use serenity's http/cache. That turned out to be unnecessary complexity:
`DISCORD_TOKEN` is already in the GUI process's environment, and a plain REST
call (`GET /guilds/{id}/channels` with `Authorization: Bot <token>`) works
without any cross-task handle, without waiting on gateway `READY`, and without
breaking when `MUNIBOT_DISABLE_BOTS=1` -- which is the documented GUI dev
workflow (`docs/gui.md`). Its 403/404 response doubles as the "bot isn't in
this guild" signal needed for an invite call to action
(`DiscordConfig::invite_link`).

If a later feature genuinely needs the serenity cache (e.g. very
high-frequency channel lookups), the option that stays architecturally clean
is cloning `client.http`/`client.cache` out of `munibot_discord::lib::start_discord_integration`
_before_ `client.start()` and publishing them from there -- not threading a
`GlobalAccess` built inside `on_ready`, since that only exists after the
gateway connects and is absent entirely with bots disabled.

## Per-user settings: the identity trap

`user_id` means two incompatible things in this schema:
`linked_accounts.user_id` is the internal `users.id` (autoincrement), while
`guild_wallets.user_id` and `guild_payouts.user_id` are raw **Discord
snowflakes with no foreign key** to anything
(`migrations/2026-02-28-.../up.sql:19,27`). A new per-user settings table
must pick one explicitly and not reuse the column name to mean both.

Recommendation: key a `user_settings` table on `users.id`, and reach it from
a bot handler via the `(provider, provider_user_id)` unique index on
`linked_accounts`. The cost of that join is fine as long as **cheap checks
run first** -- e.g. the greeting handler already regex-matches before doing
anything else (`handlers/greeting.rs:31`), so a settings lookup only runs
when someone actually triggers a greeting, not on every message. Most
Discord users have never signed into the GUI, so a per-message lookup with
no such ordering would pay a database round trip on nearly every message to
learn "no settings" almost every time.

## Linked accounts

`LinkedAccount` (`models.rs:172-183`) carries `access_token`/`refresh_token`
in the same struct as the fields safe to show a user
(`provider`, `provider_user_id`, `username`, timestamps), with **no
compile-time separation** between them. Any server function exposing linked
accounts to the client must map to a token-free DTO explicitly and have that
mapping reviewed -- the model itself won't stop a mistake.

There is currently no delete/unlink operation, no "list all linked accounts
for a user" query, and the `linked_accounts.user_id` foreign key has no
`ON DELETE CASCADE`. Since sign-in only ever reaches a `users` row through
`linked_accounts`, unlinking a user's last provider would permanently orphan
that row -- unlinking the final linked account needs to be blocked (or
cascade into deleting the user, which is a bigger decision).

## Display preferences

Timezone currently affects exactly one thing: `!magical`'s daily rollover
uses `Local::now()` (`munibot_core/src/magical.rs:13`), i.e. the server's
timezone, not the user's. Every user-facing timestamp elsewhere uses
Discord's native `<t:...>` markup (`economy.rs:144`, `logging.rs:183-191`),
which Discord already renders in each viewer's own client timezone -- a
stored timezone preference would be redundant, or actively wrong, for any of
those.

Temperature unit has a real, immediate hook: with no unit given,
`temperature.rs:29-33` currently emits _both_ conversions concatenated
together; a preferred unit would pick one. The conversion helpers
(`get_fahrenheit_to_celsius_message`, `get_celsius_to_fahrenheit_message`)
are private and Discord-only today -- Twitch has no temperature handler at
all, so moving them into `munibot_core` would let a preference apply to both
surfaces.

Pre-existing, unrelated bug noticed in passing: `munibot_twitch/src/handlers/quotes.rs:46`
stores `Local::now().naive_local()` for `quotes.created_at`, while every
other table in the schema stores UTC (`Utc::now().naive_utc()`, e.g.
`economy/payout.rs:38,73`). Worth fixing independently of any settings work;
existing rows would need a decision on whether to backfill.
</content>
