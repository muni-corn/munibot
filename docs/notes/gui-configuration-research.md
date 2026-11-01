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

**Status:** several findings below have since been acted on. Most are marked
inline rather than deleted -- the reasoning is often still the useful part
even once the bug is gone -- except the two storage bugs in the section
immediately below, which are gone outright now that they're fixed: a stale
note costs more than no note. What is still outstanding: the two autodelete
storage bugs, the missing `ON DELETE CASCADE` on `linked_accounts`, and the
quote timestamp timezone.

## Existing settings surfaces (as of the fork point)

| Scope                 | Storage             | Read                                                  | Write                                      |
| --------------------- | ------------------- | ----------------------------------------------------- | ------------------------------------------ |
| Guild logging channel | `guild_configs`     | `logging.rs:724-735`, no cache, fresh query per event | `admin.rs:65-70`, delete at `admin.rs:108` |
| Channel autodelete    | `autodelete_timers` | boot-load into a `HashMap`, `autodelete.rs:31-53`     | `autodelete.rs:55-107`, `:111-135`         |
| Global bot config     | TOML file           | `Config::read_or_write_default_from`                  | none -- only writes the default            |

At the fork point, all guild settings were Discord-slash-command only, gated
on `required_permissions = "MANAGE_GUILD"` (`admin.rs:28`). That gate is the
precedent a GUI equivalent must mirror.

**Since then**, logging settings gained a GUI surface under
`/dashboard/:guild_id` (`munibot_gui/src/pages/guild_settings/`), and the
prediction above held: it mirrors the slash-command gate through
`munibot_api/src/auth/guild.rs::require_guild_admin` rather than reusing
`is_administered_by_user`, which remains a display filter and not an
authorization check. Autodelete is still slash-command only, and the two
storage bugs below are the reason it should stay that way until they're
fixed.

## Storage bugs found by this research

Two of the four this research originally found no longer apply. `guild_configs`
only holds the logging channel, so `upsert_guild_config`'s `REPLACE INTO` and
`/admin stop-logging`'s whole-row delete both behave exactly like a
single-column write. If another setting is ever added to that row, both
become hazards again: a `REPLACE INTO` nulls every column the caller didn't
populate, and a row delete wipes every setting at once, so writes would need
to read the existing row first and merge. Two are still live:

- `AutoDeleteHandler` loads every timer into a `HashMap` once at boot
  (`autodelete.rs:30-53`) and never re-reads. A GUI (or any external) write
  straight to `autodelete_timers` would be invisible to the running bot
  until restart. Any settings surface touching autodelete-style cached state
  needs an invalidation hook, not just a database write.
- `set_autodelete` (`autodelete.rs:55-80`) always writes
  `last_cleaned: epoch` and `last_message_id_cleaned: 1` alongside the
  duration/mode. Combined with `upsert_autodelete_timer`'s `replace_into`
  (`operations.rs:142`), **editing an existing timer's duration resets its
  sweep cursor to the beginning of time**, forcing a full re-scan. Splitting
  settings columns from sweep-state columns (or at minimum, not re-writing
  sweep state on a settings-only change) fixes this.

Both are still open.

## Authorization gap

**Resolved.** This section described a real gap at the time this research was
written: nothing checked that a signed-in user actually administers the guild
they were editing. `munibot_api/src/auth/guild.rs::require_guild_admin` is that
check now.

What this section found is still accurate: `DiscordGuild::is_administered_by_user`
(`oauth/discord.rs`) remains a **display filter** in `get_guilds`, never an
authorization check on its own, and every guild-scoped settings server
function still needs its own `require_guild_admin`-style check: linked
account -> `get_current_user_guilds` -> `is_administered_by_user`, done fresh
(or short-TTL cached) per request, since there's no local cache of guild
membership to check against instead.

## Getting a channel list: the bot token, not the user's oauth token

**Resolved, exactly as recommended below.** `get_guild_channels`
(`munibot_api/src/server_fns/settings/channels.rs`) reads `DISCORD_TOKEN`
straight from the environment and calls the REST API through
`oauth/discord/bot.rs`, with no `GlobalAccess` plumbing and no dependency on
the gateway being connected. The rest of this section is kept as the
reasoning behind that choice.

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

Recommendation, **since followed**: key a `user_settings` table on `users.id`,
and reach it from a bot handler via the `(provider, provider_user_id)` unique
index on `linked_accounts`. `ai_user_settings` and every other `ai_*` table
does exactly this, with a real foreign key and `ON DELETE CASCADE`. The cost
of that join is fine as long as **cheap checks
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

**Partly resolved.** `list_linked_accounts` and `unlink_linked_account` both
exist now (`operations.rs:650` and below), and unlinking the final linked
account **is** blocked -- the "not the last one" check the paragraph below
asked for. The `/account` page is the user-facing surface.

Still open: `linked_accounts.user_id` has no `ON DELETE CASCADE`
(`migrations/2026-07-10-.../up.sql:24`), so deleting a `users` row would
orphan its links. Not reachable today, since nothing deletes a user, but the
constraint is still missing.

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
