# Milestone 2 — chat product

**Outcome:** munibot remembers you, picks the right persona on his own, works on Twitch as well as
Discord, and can be configured and audited through the web interface.

Milestone 1 proved the harness. This milestone turns it into something people can actually live with:
conversations survive restarts, munibot remembers what matters to you if you let him, and you can see
what he is costing.

**Phases 9 through 13, commits 67 through 99.**

---

## Phase 9 — persistence

The diesel-backed session store. Migrations live at the workspace root in `migrations/`, embedded and
applied at startup by the existing `run_pending_migrations()`.

| #   | Commit                                             | Description                                                                                                                                                                                                                                                                                                           |
| --- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 67  | `feat(db): add ai conversation and message tables` | Migration creating `ai_conversations` and `ai_messages` as specified in the overview. `ai_messages.content` is JSON holding `Vec<ContentBlock>`. Unique index on `(platform, scope_key)` and on `(conversation_id, seq)`. Regenerate `munibot_core/src/db/schema.rs` with `diesel print-schema`.                      |
| 68  | `feat(db): add ai usage and tool call tables`      | Migration creating `ai_usage` and `ai_tool_calls`. Index `ai_usage` on `(guild_id, created_at)` and `(user_id, created_at)`, because every budget query and every dashboard panel filters on exactly those.                                                                                                           |
| 69  | `feat(core): add ai conversation models`           | `Queryable`/`Selectable` and `Insertable` structs in `munibot_core/src/db/models.rs` for the new tables, each with `#[diesel(check_for_backend(diesel::mysql::Mysql))]`, following the existing conventions.                                                                                                          |
| 70  | `feat(core): add ai conversation operations`       | Free async functions in a new `munibot_core/src/db/operations/ai.rs`, taking `&DbPool` and returning `QueryResult<T>`. MySQL has no `RETURNING`, so inserts use the existing `last_insert_id()` helper at `operations.rs:21`. Split into a submodule because `operations.rs` is already 569 lines.                    |
| 71  | `feat(ai_memory): add diesel session store`        | `DieselSessionStore` implementing `SessionStore` over `DbPool`, behind a `diesel` feature that pulls in `munibot_core`. Integration tests use the `TestDb` fixture at `munibot_core/tests/common/mod.rs:32`.                                                                                                          |
| 72  | `feat(ai_memory): add conversation summarisation`  | `Summariser` taking a provider and a compaction persona, condensing the oldest messages into prose when history exceeds a token threshold, writing it to `ai_conversations.summary`, and deleting the messages it replaced. Triggered from `assemble_context` rather than on a timer, so it only costs when it helps. |
| 73  | `feat(ai): add usage recording after every turn`   | Write an `ai_usage` row on turn completion with resolved provider, model, token counts, and estimated cost. Record on failure too, since a turn that errored on iteration nine still cost money.                                                                                                                      |
| 74  | `feat(ai): add tool call auditing`                 | Write an `ai_tool_calls` row per invocation with truncated input and output, duration, and status. Powers the transcript viewer in phase 13 and is the only way to debug a bad tool loop after the fact.                                                                                                              |

---

## Phase 10 — user memory

Opt-in, user-controlled, and scoped to the internal `users.id` so it survives a second linked
account.

| #   | Commit                                                    | Description                                                                                                                                                                                                                                                                                               |
| --- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 75  | `feat(db): add ai memory and user settings tables`        | Migration creating `ai_memories` and `ai_user_settings`, both with a real foreign key to `users.id` and `ON DELETE CASCADE`, so deleting a user erases their memories automatically. Unique index on `(user_id, key)`.                                                                                    |
| 76  | `feat(ai_memory): add memory store trait and diesel impl` | `MemoryStore` with `list(user_id)`, `record(user_id, key, value)`, `forget(user_id, key)`, and `wipe(user_id)`. `record` upserts on the unique key using the `on_conflict(DuplicatedKeys)` pattern documented at `operations.rs:33`. A per-user memory cap prevents unbounded growth.                     |
| 77  | `feat(ai_memory): add memory opt in gating`               | Every read and write path checks `ai_user_settings.memory_opt_in` first and returns empty or a clear refusal when it is unset. Default is off. Enforced in the store, not the caller, so no future caller can forget.                                                                                     |
| 78  | `feat(ai_tools): add remember and forget tools`           | `remember` taking `key` and `value`, and `forget` taking `key`. Tier `Safe`, but both refuse when the invoker has not opted in, returning a `ToolOutcome::Err` telling the model to mention `/memory enable`. There is deliberately no recall tool — retrieval is the host's job, not the model's.        |
| 79  | `feat(ai): add memory injection into system prompts`      | Load the invoker's memories before the turn and render them into the system prompt through a `{{memories}}` template variable. Personas with `MemoryPolicy::User` get them, others do not. Keeping retrieval out of the model's hands makes it predictable and costs one query instead of a round-trip.   |
| 80  | `feat(discord): add memory management commands`           | `/memory enable`, `/memory disable`, `/memory list`, `/memory delete key:<key>`, and `/memory wipe` with a confirmation button. Every response is ephemeral. `disable` keeps the rows but stops all use; `wipe` deletes them. Being able to see and delete everything is the point of the opt-in promise. |

---

## Phase 11 — automatic persona routing

Sticky routing: classify once per conversation, not once per message.

| #   | Commit                                                  | Description                                                                                                                                                                                                                                                                                              |
| --- | ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 81  | `feat(ai): add router configuration and decision types` | `RouterConfig { enabled, model, sticky, confidence_threshold }` and `RouteDecision { persona, confidence, reason }`. Add a `pinned_persona` column to `ai_conversations` in the same commit as the code that reads it.                                                                                   |
| 82  | `feat(ai): add router prompt and persona catalogue`     | `router.md` receiving the message, the current persona, and a `{{personas}}` catalogue rendered from each persona's `description`. Returns a `RouteDecision` through the harness handoff mechanism, which is exactly what handoff was built for.                                                         |
| 83  | `feat(ai): add sticky routing resolution`               | `resolve_persona` precedence: an explicit request wins, then a pinned channel persona, then a sticky conversation persona unless the router reports a topic change above threshold, then the router, then the default. Pure function over a small state struct with table-driven tests for every branch. |
| 84  | `feat(ai): add router failure fallback`                 | A router error, a timeout, or a below-threshold decision falls back to the current or default persona and logs at `warn`. The router is never allowed to break a conversation, and its cost is capped by a hard iteration limit of one.                                                                  |
| 85  | `feat(discord): add routing decision transparency`      | When the router changes persona, note it quietly in the response footer. Users get very confused by an invisible personality switch, and this also makes router quality debuggable in production.                                                                                                        |

---

## Phase 12 — Twitch adapter

No message editing on Twitch, so the rendering strategy is completely different: buffer, then chunk.

| #   | Commit                                            | Description                                                                                                                                                                                                                                                                                                  |
| --- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 86  | `feat(twitch): add ai chat message handler`       | `munibot_twitch/src/handlers/ai.rs` implementing `TwitchMessageHandler`, returning `Ok(true)` only when it handled the message. Triggers on a `munibot,` prefix or a configured command. Conversation scope is the channel, so chat shares one context.                                                      |
| 87  | `feat(twitch): add buffered response chunking`    | Collect the full response, then split at 480 characters on sentence and word boundaries, sending sequentially with a small delay. Cap the total at a configurable number of messages so a long answer cannot flood a channel.                                                                                |
| 88  | `feat(twitch): add slow response acknowledgement` | If no response has arrived within roughly three seconds, send a short holding message. Without it, chat assumes the bot is broken and asks again, which doubles the cost.                                                                                                                                    |
| 89  | `feat(twitch): add per channel ai enablement`     | Only respond in channels that have opted in. Replaces the hardcoded `muni_corn` gate at `munibot_twitch/src/bot.rs:144` for this handler specifically, using a per-channel configuration lookup with an in-memory cache that is invalidated on change, avoiding the stale-cache bug documented in the notes. |
| 90  | `feat(twitch): register ai handler in the bot`    | Add the handler to `TwitchHandlerCollection` in `munibot_twitch/src/bot.rs:37`, ordered after existing command handlers so it never shadows a real command.                                                                                                                                                  |

---

## Phase 13 — API and web interface

Follows the logging settings vertical slice, which is the newest and most complete example in the
repository. Read `docs/notes/gui-configuration-research.md` before starting: it documents an
authorization gap where `HasPermission::has` always returns `false`, and the reason channel listing
needs the bot token rather than the user's OAuth token.

| #   | Commit                                                   | Description                                                                                                                                                                                                                                     |
| --- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 91  | `feat(db): add ai guild settings columns`                | Add `ai_enabled`, `ai_default_persona`, and `ai_channel_mode` to `guild_configs`, plus an `ai_channel_allowlist` table. Use the whole-row upsert pattern at `operations.rs:33`, never `REPLACE INTO`, which previously wiped unrelated columns. |
| 92  | `feat(api): add ai settings data transfer objects`       | Wire types in `munibot_api/src/settings/ai.rs` plus an `AiSettingsError` with an `AsStatusCode` impl, mirroring `munibot_api/src/settings/error.rs:12`. Types before the functions that use them.                                               |
| 93  | `feat(api): add ai settings server functions`            | `get_ai_settings` and `set_ai_settings` using the `#[server(name: Type)]` extractor pattern documented at `docs/gui.md:72`. Both call the guild-admin check at `munibot_api/src/auth/guild.rs:20` before touching the database.                 |
| 94  | `feat(api): add ai usage summary server function`        | `get_ai_usage(guild_id, range)` aggregating `ai_usage` into totals by day, model, and persona. Guild-admin gated. Aggregate in SQL, not in Rust, or this gets slow within a month.                                                              |
| 95  | `feat(gui): add ai settings page`                        | A settings page under the existing guild settings layout: an enable toggle, a default persona selector fed from the registry, and a channel allowlist editor. Registered in the route table at `munibot_gui/src/app.rs:37`.                     |
| 96  | `feat(gui): add ai usage dashboard`                      | Spend over time, token totals, and a breakdown by persona and model. **This is the panel that keeps the project affordable**, so it lands before public exposure rather than after.                                                             |
| 97  | `feat(api): add conversation transcript server function` | `get_ai_transcript(conversation_id)` returning messages with their tool calls, guild-admin gated, with the bot's own reasoning blocks stripped. Paginated from the start.                                                                       |
| 98  | `feat(gui): add conversation transcript viewer`          | Render a transcript with tool calls collapsible and their inputs and outputs inspectable. The fastest way to understand why a persona behaved oddly.                                                                                            |
| 99  | `feat(gui): add persona catalogue page`                  | A read-only listing of configured personas with descriptions, models, and tool allowlists. Makes the configuration legible without shell access, and doubles as user-facing documentation for what munibot can do.                              |

---

## Definition of done

- A conversation survives a restart with full context.
- Long conversations compact themselves instead of erroring on context length.
- Opting in, recording a fact, and having munibot use it next week works end to end.
- `/memory wipe` removes everything, verifiably, through the transcript viewer.
- Asking a coding question and then a feelings question routes to different personas, visibly.
- The same conversation works on Twitch, chunked sensibly.
- The usage dashboard shows real spend per guild.

## Deliberately deferred

No sandbox, no repository access, no GitHub integration. Milestones 3 and 4.
