# Milestone 04 — affordability

**Status:** full recipe. Start after [milestone 03](milestone-03-conversations.md).

**Outcome:** nobody can spend more than you allow. Every turn is checked before it starts against
per-user request, concurrency, and spend limits, plus a global daily cap. A turn that's refused gets
a kind, specific message saying when the limit resets. Each person can see what they've spent on a
`/usage` page. Operators can see global spend there, and can pause all chat with a switch that
survives a restart.

This milestone also introduces **operators**, the first real permission in munibot. Today, the only
authorization in the codebase is "signed in" and `require_guild_admin`, which asks Discord. Spend
controls are the first thing that needs to know who's in charge.

---

## Starting point

This milestone assumes milestone 03 is done:

- `ai_usage` gets a row for every turn, with `model_cost_micros` and `tool_cost_micros`, indexed on
  `(user_id, created_at)` and `(created_at)`.
- `sum_cost_since(user_id, since)` exists.
- `Chat::turn` takes a per-conversation lock and runs a budgeted harness turn.

From the code at `9b4a38e1`:

- There's no permission system at all. `HasPermission`, `require_operator`, a `Permission` enum, and
  `docs/notes/permission-system.md` are all cited by the old plans, and none of them exist.
- `munibot_core::Config` holds only `discord` and `twitch` (`munibot_core/src/config.rs:8`). `[ai]` is
  parsed separately by `AiConfig::load_from_file`.
- `munibot_core` doesn't depend on `strum`. `munibot_discord` and `munibot_twitch` do.
- `UserData { display_name, avatar_url }` is all the GUI knows about the signed-in user
  (`munibot_api/src/auth.rs:15`).

## Design notes

**Limits are computed from `ai_usage`, not stored separately.** The old plan had dedicated
`ai_rate_limits` and `ai_spend_caps` tables, but milestone 03 already records every turn with its
cost, indexed by user and time. "How many turns in the last hour" and "how much spent today" are
single indexed aggregates over that table. A second copy of the same facts would only drift. The one
gap is turns that are still running, which haven't written a row yet. An in-memory tracker covers
them.

**Admission reserves a turn's worst case.** A turn's budget has a `max_cost`. Admission checks
`spent + reserved + this turn's max_cost <= cap`, and holds that reservation until the turn ends. So
several concurrent turns can never jointly overshoot a cap, which a "check what's spent so far"
design allows. It's conservative on purpose: someone very close to their cap is refused a turn that
might have been cheap, and the message says why.

**Operators come from config, and config can take them away.**

```toml
[[operators]]
provider = "discord"
provider_user_id = "123456789012345678"

[[operators]]
user_id = 1
```

`user_permissions` records each grant's `source`, either `config` or `manual`. Sync is set-based:
grant `config` rows to every matching user, and delete `config` rows for every user who no longer
matches. `manual` grants are never touched by sync. This answers the question the old plan deferred,
about what "no longer matches" means when someone unlinks an account or leaves the config: sync owns
exactly the rows it created.

**The pause switch is persisted.** If a kill switch resets on restart, then restarting quietly undoes
it. It lives in a single-row `ai_runtime_settings` table, and admission reads it through a cache that
expires every few seconds.

**Operators skip per-user limits but not the global cap.** The global cap exists to protect the
provider key, so it applies to everyone.

---

## Schema

```
user_permissions     (user_id BIGINT -> users ON DELETE CASCADE, permission VARCHAR(32),
                      source VARCHAR(16), granted_at, PRIMARY KEY (user_id, permission))

ai_runtime_settings  (id TINYINT PK CHECK (id = 1), chat_paused BOOL NOT NULL DEFAULT FALSE,
                      paused_by_user_id NULL -> users ON DELETE SET NULL, paused_at NULL)
```

## Configuration

```toml
[ai.limits]
# per signed-in user; operators are exempt from these four
turns_per_hour = 60
max_concurrent_turns = 2
user_daily_usd = 1.00
user_monthly_usd = 10.00
# everyone, operators included
global_daily_usd = 20.00
```

Every field is optional and has the default shown. Limits require the turn budget to have a
`max_cost`. If `[ai.budget].max_cost_usd` is removed while limits are on, startup fails with a message
that says so.

---

## Commits

### Permissions

#### 4.1 Add the permission type

- `cargo add -p munibot_core strum -F derive`.
- `munibot_core/src/permission.rs`: `Permission` has one variant, `Operator`. `PermissionSource` is
  `Config` or `Manual`. Both derive strum's snake-case `Display` and `EnumString`.

```gitcommit
feat(core): add the permission type

munibot has had no notion of who is in charge. one permission is
enough to start: operators. a grant's source is recorded so that
config can revoke exactly what config granted.
```

#### 4.2 Add the user permissions table

Add a migration, the schema, and the `UserPermission`/`NewUserPermission` models.

```gitcommit
feat(db): add the user permissions table

one row per permission a user holds, with where it came from. it
cascades with its user like every other per-user table.
```

#### 4.3 Add permission operations

`munibot_core/src/db/operations/permissions.rs`:

- `grant(user, permission, source)`, upserting with `on_conflict`.
- `revoke(user, permission)`.
- `list_for_user`.
- `has_permission`.
- `find_user_by_linked_account(provider, provider_user_id)`.
- `sync_config_grants(permission, desired: &[i64])`, which grants missing rows and deletes stale
  `config` rows in one transaction.

Integration tests cover sync leaving `manual` grants alone.

```gitcommit
feat(db): add permission operations

sync is set-based and transactional: everyone config names gets the
grant, everyone it no longer names loses the grant config gave them,
and nothing granted by hand is touched.
```

#### 4.4 Add operators to the config

`munibot_core::Config.operators: Vec<OperatorRef>`, where `OperatorRef` is an untagged enum of
`{ provider, provider_user_id }` or `{ user_id }`. It defaults to empty, and an existing config file
keeps loading.

```gitcommit
feat(config): add operators to the config file

operators are named in config by any linked sign-in or by munibot user
id, so whoever deploys munibot decides who runs it.
```

#### 4.5 Sync operators at startup

`munibot/src/permissions.rs`: resolve each `OperatorRef` to a user id, call
`sync_config_grants(Operator, …)`, and log how many grants were added and removed. A reference that
matches nobody gets a `warn!` naming it. This runs after migrations and before the server starts.

```gitcommit
feat(munibot): sync operators from config at startup

removing someone from the operators list now actually removes their
operator access, instead of only ever granting.
```

#### 4.6 Resync operators after sign-in

After any successful sign-in or account link, run the same sync for that one user, so someone named
by a provider account they only just linked becomes an operator without a restart.

```gitcommit
feat(api): resync operator access after signing in or linking

a config entry can name an account that wasn't linked when munibot
started. resyncing on sign-in picks it up immediately.
```

#### 4.7 Add the operator check

- `munibot_api/src/auth/operator.rs`: `require_operator(&AuthSession, &DbPool) -> Result<User, AuthError>`.
- `AuthError::NotOperator`, which maps to 403.
- `UserData.is_operator`, so the GUI can show operator-only links.

```gitcommit
feat(api): add the operator check

server functions that expose global data or controls need one line
to refuse anyone who isn't an operator. the gui also needs to know,
so it doesn't offer links that would only be refused.
```

### Configuration and runtime settings

#### 4.8 Add the runtime settings table

Add a migration creating the single-row `ai_runtime_settings` table, seeded with `chat_paused = false`,
plus `get_runtime_settings` and `set_chat_paused(paused, by_user)`.

```gitcommit
feat(db): add the ai runtime settings table

holds the chat pause switch. a kill switch that resets on restart
could be undone by accident, so it lives in the database.
```

#### 4.9 Add the limits section

`AiConfig.limits: LimitsConfig`, with the defaults above, plus a validation function that requires a
budget `max_cost` whenever limits apply.

```gitcommit
feat(config): add per-user and global spend limits

turns, concurrency, and spend can now be capped per person, and total
spend per day. the defaults are deliberately small, so an unconfigured
public deployment fails safe.
```

### Admission

#### 4.10 Add limit types and the usage source

`munibot_ai/src/limits.rs`, with `limits/`:

- `LimitRefusal` is `TooManyTurns`, `TooManyConcurrent`, `UserDailySpend`, `UserMonthlySpend`,
  `GlobalDailySpend`, or `Paused`. Each carries `resets_at: Option<DateTime<Utc>>`, and `Display`
  produces the friendly sentence.
- A `UsageSource` trait with `turns_since`, `cost_since`, and `paused`, plus a diesel implementation
  over `ai_usage` and `ai_runtime_settings`.
- `AiError::Limited(LimitRefusal)`.

```gitcommit
feat(ai): add limit refusals and the usage source

every refusal says which limit was hit and when it resets, in words
someone can act on. usage is read from the turns already recorded,
not from a second set of counters that could drift.
```

#### 4.11 Add admission control

`limits/admission.rs`: `Admission::admit(user, is_operator, max_cost) -> Result<TurnPermit, LimitRefusal>`.

- Check order: paused, global daily, then per-user concurrency, hourly turns, daily spend, and
  monthly spend. Operators skip the per-user checks.
- In-flight turns and their reservations are held in memory. `TurnPermit` releases both on drop.
- Logs a `warn!` when any scope passes 80% of its cap.
- Tested against a fake source with concurrent admissions.

```gitcommit
feat(ai): add turn admission control

decides before any provider call whether a turn may run. each running
turn reserves its own worst-case cost, so concurrent turns can never
overshoot a cap together, and the global cap is checked first because
it is the last line of defence.
```

#### 4.12 Check limits before every turn

`Chat::turn` takes `is_operator` on its context and admits before appending or calling anything. A
refusal returns `AiError::Limited` and writes nothing. The permit lives as long as the turn's stream.
`chat_stream`, `send_message`, and `retry_reply` fill in `is_operator`.

```gitcommit
feat(ai): check limits before every chat turn

a refused turn costs nothing and leaves nothing behind, not even the
message, so trying again later doesn't produce a duplicate.
```

### Surfaces

#### 4.13 Carry limit refusals to the page

`ChatError::Limited { message, resets_at }` maps to 429.

```gitcommit
feat(chat): carry limit refusals to the chat page

the page has to tell a limit apart from an outage. one deserves a
retry button and the other deserves a time to come back.
```

#### 4.14 Explain limits kindly

The banner shows the refusal's sentence and a relative reset time ("in about 3 hours"), keeps the
draft in the composer, and offers no retry button until the reset time has passed.

```gitcommit
feat(gui): explain limit refusals kindly

being refused by a companion should feel like a pause, not an error.
saying exactly when it lifts is most of that.
```

#### 4.15 Add usage server functions

- `get_my_usage()` returns today's and this month's spend against the caller's caps, turns this hour,
  and recent turns with conversation titles.
- `get_global_usage()` (operator only) returns global spend today and this month, the top users by
  spend, and the paused state.
- `set_chat_paused(paused)` (operator only).
- Everything is aggregated in SQL.

```gitcommit
feat(api): add usage server functions

people can see their own spend, and operators can see everyone's and
stop all chat at once. aggregation stays in sql, so the page stays
fast as the usage table grows.
```

#### 4.16 Add the usage page

`/usage`, linked from the account menu. It shows spend against each cap as progress bars, flags 80%
or more, and lists recent turns with their cost.

```gitcommit
feat(gui): add the usage page

showing people what they have spent is honest, and it is the cheapest
abuse deterrent there is.
```

#### 4.17 Add operator controls

An operator section on `/usage`: global spend, top users, and a pause toggle that asks for
confirmation.

```gitcommit
feat(gui): add operator usage and pause controls

the view that catches a problem across everyone, and the switch that
stops it, on a page that works from a phone.
```

#### 4.18 Start the operations runbook

`docs/ai-operations.md`: how to name operators, every `[ai.limits]` key, what each refusal means,
how to pause and unpause, and what to check when spend jumps. Update `docs/chat.md` to drop its "no
rate limits" warning and link here.

```gitcommit
docs: start the ai operations runbook

the first ai document written for whoever runs munibot rather than
whoever develops it. later milestones add to it.
```

---

## Definition of done

- A user's sixty-first turn in an hour is refused with a message naming when the limit resets, and
  their message isn't saved.
- Three turns started at once by a user with `max_concurrent_turns = 2` admit two and refuse one.
- With `user_daily_usd` set just above one turn's `max_cost`, the second turn of the day is refused
  before any provider call.
- Pausing chat refuses every new turn, including an operator's, and still does after a restart.
- Removing someone from `[[operators]]` and restarting removes their operator access. A manual
  grant survives the same restart.
- `/usage` shows a person their own spend. Only operators see the global section.

## Deliberately deferred

- **Per-guild limits.** There is no guild-scoped AI surface until milestone 12, which adds guild
  limits along with it.
- **Abuse detection.** Repeated identical prompts, injection signatures, and escalating cooldowns are
  milestone 11. Limits bound the damage; they don't diagnose it.
- **Granting permissions from the GUI.** `manual` grants exist in the table, but are only written by
  hand for now. A permissions page belongs with the operator tooling in milestone 11.
- **The public gate.** Spend is bounded now, but the companion still lacks the care path.
  [The overview](overview.md#before-going-public) lists everything that has to exist before `/chat`
  is opened to strangers.
