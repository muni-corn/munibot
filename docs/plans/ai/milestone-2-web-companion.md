# Milestone 2 — the companion on the web

**Outcome:** anyone signed in can open munibot's own chat page and have a real, continuing
conversation with a companion who remembers them — and who is genuinely useful for programming and
research along the way.

Milestone 1 proved the harness in Discord. This milestone makes the **web interface the primary
surface** for munibot's AI, because that is where the companion can actually be shown off: a
persistent conversation list, streamed replies, rendered code, visible tool work, and a memory panel
you can inspect and empty. Discord keeps what phase 8 already built and stops there.

munibot is a companion first. Everything here is ordered so the companion works well before anything
clever gets added on top, and phase 13 exists specifically to make him worth the title rather than
merely functional.

**Phases 9 through 14, commits 67 through 108.**

---

## What this milestone has to overcome

Three facts about the current codebase shape every phase below, and all three were confirmed by
reading it rather than assumed:

- **`munibot_api` and `munibot_gui` do not know `munibot_ai` exists.** Neither `Cargo.toml` mentions
  it and neither `src` tree references it. `munibot_ai` is reachable only from the `munibot` binary.
- **The `Ai` service is built inside the bot-startup guard.** `munibot/src/main.rs:60-73` constructs
  it inside `if std::env::var("MUNIBOT_DISABLE_BOTS").is_err()` and moves it into
  `munibot::bot::start`. But `MUNIBOT_DISABLE_BOTS=1` is _the documented GUI development workflow_
  (`docs/gui.md:148-150`), so until this is restructured the chat page cannot be developed locally at
  all. `munibot_gui::server::run` also currently takes only a `DiscordConfig`.
- **`HarnessEvent` cannot cross the wire.** `munibot_ai/src/harness/event.rs:12` derives `Debug`
  alone, and the enum carries an `AiError` and a `serde_json::Value`. A wire DTO in `munibot_api` is
  the consistent fix, matching how `munibot_core` models are already translated into slim DTOs rather
  than shared directly (`docs/gui.md:33-35`).

One piece of good news: `Platform::Web` already exists in `munibot_ai/src/tools/context.rs`, and
`ConversationScope` is already platform-keyed, so no enum needs widening.

---

## Phase 9 — persistence

The diesel-backed session store, plus the ownership and titling that a per-user conversation list
needs. Migrations live at the workspace root in `migrations/`, embedded and applied at startup by the
existing `run_pending_migrations()`.

Two findings from implementing this phase, both worth knowing before the phases that depend on them:

- **The `TestDb` fixture was broken before any of this started.** It hardcoded
  `root:sillylittlepassword@127.0.0.1:3306`, which this project's devenv never provides — it falls back
  to port 3307 when 3306 is taken, and its root user has no password. All 31 existing integration tests
  failed with a bare connection error. Both URLs are now overridable via `MUNIBOT_TEST_ROOT_DB_URL` and
  `MUNIBOT_TEST_DB_BASE_URL`, which was a prerequisite commit for this phase rather than part of it.
- **A user cannot be deleted while a `linked_accounts` row references them.** That older foreign key has
  no `ON DELETE CASCADE`, unlike the ones added here. Phase 10 promises that deleting a user erases
  their memories automatically; delivering that promise needs either that foreign key fixed or an
  explicitly ordered delete.

| #   | Commit                                             | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 67  | `feat(db): add ai conversation and message tables` | Migration creating `ai_conversations` and `ai_messages`. `ai_messages.content` is JSON holding `Vec<ContentBlock>`. Unique index on `(platform, scope_key)` and on `(conversation_id, seq)`. **`ai_conversations` carries `owner_user_id`, `title`, and `archived_at` from the start**, rather than bolting them on later: a web conversation belongs to one person and needs a name in a sidebar, while a Discord channel's conversation has neither. `owner_user_id` is a real foreign key to `users.id` with `ON DELETE CASCADE`, and is `NULL` for channel-scoped conversations. |
| 68  | `feat(db): add ai usage and tool call tables`      | Migration creating `ai_usage` and `ai_tool_calls`. Index `ai_usage` on `(user_id, created_at)` first and `(guild_id, created_at)` second — with the web as the primary surface, per-user is now the common query and per-guild the secondary one.                                                                                                                                                                                                                                                                                                                                    |
| 69  | `feat(core): add ai conversation models`           | `Queryable`/`Selectable` and `Insertable` structs in `munibot_core/src/db/models.rs`, each with `#[diesel(check_for_backend(diesel::mysql::Mysql))]`, following existing conventions.                                                                                                                                                                                                                                                                                                                                                                                                |
| 70  | `feat(core): add ai conversation operations`       | Free async functions in a new `munibot_core/src/db/operations/ai.rs`, taking `&DbPool` and returning `QueryResult<T>`. MySQL has no `RETURNING`, so inserts use the existing `last_insert_id()` helper at `operations.rs:21`. A submodule because `operations.rs` is already 569 lines.                                                                                                                                                                                                                                                                                              |
| 71  | `feat(memory): add diesel session store`           | `DieselSessionStore` implementing the existing `SessionStore` trait over `munibot_core::DbPool`. No feature gate needed — `munibot_ai` has depended on `munibot_core` unconditionally since phase 7's `AiConfig`. Integration tests use the `TestDb` fixture at `munibot_core/tests/common/mod.rs:32`.                                                                                                                                                                                                                                                                               |
| 72  | `feat(memory): add conversation directory`         | A `ConversationDirectory` trait and diesel implementation: `list_for_user`, `create_for_user`, `rename`, `archive`. Deliberately **separate from `SessionStore`**, which is about one scope's message history — listing a person's conversations is a different question with a different index, and folding both into one trait would force every future store to implement ownership semantics it may not have.                                                                                                                                                                    |
| 73  | `feat(memory): add conversation summarisation`     | `Summariser` taking a provider and a compaction persona, condensing the oldest messages into prose once history exceeds a token threshold, writing it to `ai_conversations.summary`, and deleting the messages it replaced. Triggered from `assemble_context` rather than on a timer, so it only costs when it helps.                                                                                                                                                                                                                                                                |
| 74  | `feat(ai): add usage recording after every turn`   | Write an `ai_usage` row on turn completion with resolved provider, model, token counts, and estimated cost. Record on failure too, since a turn that errored on iteration nine still cost money.                                                                                                                                                                                                                                                                                                                                                                                     |
| 75  | `feat(ai): add tool call auditing`                 | Write an `ai_tool_calls` row per invocation with truncated input and output, duration, and status. The only way to debug a bad tool loop after the fact, and what the tool activity display in phase 12 reads back.                                                                                                                                                                                                                                                                                                                                                                  |

---

## Phase 10 — memory

Opt-in, user-controlled, and scoped to the internal `users.id` so it survives a second linked account.
This is the phase that turns "a chatbot" into "someone who knows you", so it lands before the
interface rather than after.

Note the identity trap documented at `docs/notes/gui-configuration-research.md:91-108`:
`linked_accounts.user_id` holds the internal `users.id`, while `guild_wallets.user_id` holds a raw
Discord snowflake with no foreign key. Every table here keys on `users.id`, explicitly.

| #   | Commit                                                 | Description                                                                                                                                                                                                                                                                                                   |
| --- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 76  | `feat(db): add ai memory and user settings tables`     | Migration creating `ai_memories` and `ai_user_settings`, both with a real foreign key to `users.id` and `ON DELETE CASCADE`, so deleting a user erases their memories automatically. Unique index on `(user_id, key)`.                                                                                        |
| 77  | `feat(memory): add memory store trait and diesel impl` | `MemoryStore` with `list(user_id)`, `record(user_id, key, value)`, `forget(user_id, key)`, and `wipe(user_id)`. `record` upserts on the unique key using the `on_conflict(DuplicatedKeys)` pattern documented at `operations.rs:33`. A per-user memory cap prevents unbounded growth.                         |
| 78  | `feat(memory): add memory opt in gating`               | Every read and write path checks `ai_user_settings.memory_opt_in` first and returns empty or a clear refusal when it is unset. Default is off. Enforced **in the store, not the caller**, so no future caller can forget.                                                                                     |
| 79  | `feat(tools): add remember and forget tools`           | `remember` taking `key` and `value`, and `forget` taking `key`. Tier `Safe`, but both refuse when the invoker has not opted in, returning a `ToolOutcome::Err` telling the model to point at the memory panel. There is deliberately no recall tool — retrieval is the host's job, not the model's.           |
| 80  | `feat(ai): add memory injection into system prompts`   | Load the invoker's memories before the turn and render them into the system prompt through a `{{memories}}` template variable. Personas with `MemoryPolicy::User` get them, others do not. Keeping retrieval out of the model's hands makes it predictable and costs one query instead of a whole round trip. |

Phase 8's Discord `/memory` commands are **not** part of this plan any more. Memory management moves
to the web panel in phase 13, where it can show everything at once instead of paginating through
ephemeral replies.

---

## Phase 11 — the AI web API

The structural phase: make `Ai` reachable from the GUI server, define the wire types, and expose a
streamed turn. Nothing here renders anything.

Read `docs/gui.md:72-100` before starting. The `#[server(name: Type)]` extractor pattern hoists
extractor arguments out of the client stub entirely, which is why the attribute may name types
(`axum`, `crate::auth::server`) that do not exist in the wasm build — **as long as they are referenced
by full path inside the attribute and never through a top-level `use`**.

| #   | Commit                                                                     | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 81  | `build(api): add ai and stream dependencies to the api crate`              | `munibot_ai` as a `server`-feature-only optional dependency, mirroring how `munibot_core` is treated at `munibot_api/Cargo.toml:19`. `futures` is added **unconditionally**, since the stream payload types have to compile for wasm too.                                                                                                                                                                                                                                                                                             |
| 82  | `refactor(munibot): construct the ai service independently of bot startup` | Hoist `Ai` construction out of the `MUNIBOT_DISABLE_BOTS` guard in `munibot/src/main.rs:60-73`, and pass `Option<Arc<Ai>>` into `munibot_gui::server::run` so it can be layered as an `Extension` beside the pool and Discord config at `munibot_gui/src/server.rs:57-58`. **Prerequisite for developing any of phase 12 locally**, since the documented GUI workflow sets that variable and would otherwise leave the chat page with no service behind it.                                                                           |
| 83  | `feat(api): add chat wire types`                                           | `munibot_api/src/chat/`: `ConversationSummary`, `ChatMessage`, `ChatRole`, `ChatEvent`, and `PersonaSummary`. `ChatEvent` is the serializable mirror of `HarnessEvent`, which derives only `Debug` and carries non-serializable payloads. The `HarnessEvent -> ChatEvent` mapping is a pure function and is unit-tested directly — the one genuinely testable part of this phase.                                                                                                                                                     |
| 84  | `feat(api): add chat error type`                                           | `ChatError` with an `AsStatusCode` impl and `#[cfg(feature = "server")]`-gated `From` impls for `munibot_ai::AiError`, `anyhow::Error`, and `diesel::result::Error`, mirroring `munibot_api/src/settings/error.rs:12-87`. Variants are distinguishable so the GUI can match structurally rather than only printing a string, the way `SettingsError::BotNotInGuild` drives its own UI at `guild_settings/logging.rs:108`.                                                                                                             |
| 85  | `feat(api): add conversation server functions`                             | `list_conversations`, `create_conversation`, `get_conversation_messages` (paginated), `rename_conversation`, `archive_conversation`. Each resolves the caller via `auth.current_user` and **refuses any conversation whose `owner_user_id` is not theirs** — ownership is checked per call, never inferred from possession of an id. Deliberately _not_ guild-gated: `require_guild_admin` costs a live Discord HTTP round trip per call (`munibot_api/src/auth/guild.rs:14-19`), which a chat surface must not pay on every message. |
| 86  | `feat(api): add message submission server function`                        | `send_message(conversation_id, text, persona)` persisting the user's message and returning a turn identifier. Split from streaming on purpose: SSE is a `GET`, and putting a pasted code block in a query string would hit URL length limits exactly when the coding use case needs it most.                                                                                                                                                                                                                                          |
| 87  | `feat(api): add chat streaming endpoint`                                   | `#[get("/api/ai/chat/stream", auth: ..., ai: ...)]` returning `ServerEvents<ChatEvent>` built with `ServerEvents::from_stream` over `Ai::turn_streamed`. `#[server]` hard-codes `POST`, so a `#[get]` route is required; the `name: Type` extractor syntax parses identically on it. SSE rather than a websocket: the client sends exactly one message per turn, so duplex buys nothing, while SSE reconnects trivially and is readable in devtools.                                                                                  |
| 88  | `feat(api): add persona and memory server functions`                       | `list_personas` so the picker and the catalogue page share one source of truth, plus `get_memory_settings`, `set_memory_opt_in`, `list_memories`, `forget_memory`, and `wipe_memories` for the phase 13 panel.                                                                                                                                                                                                                                                                                                                        |

---

## Phase 12 — the chat page

munibot's own page, not a settings screen. A new top-level route rather than something nested under
`Dashboard`, whose sidebar is guild-scoped and irrelevant here.

Follow the vertical slice at `munibot_gui/src/pages/guild_settings/logging.rs:19-134`: `use_resource`
per read, `use_signal` for form state, `use_effect` to seed from a resolved resource, `spawn` for
mutations, and an exhaustive `match` on `&*resource.read()` assigned to a `content` binding. Styling is
tailwind 4 plus daisyUI 5; the five components in `munibot_gui/src/components/settings.rs` are the
entire existing design system, so chat primitives are new ground and belong in
`munibot_gui/src/components/chat/`.

| #   | Commit                                                | Description                                                                                                                                                                                                                                                                                                                                                                                      |
| --- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 89  | `feat(gui): add chat route and layout`                | `Route::Chat` and `Route::ChatConversation { conversation_id }` in the table at `munibot_gui/src/app.rs:37`, under `MainLayout` and `HomeLayout` but **not** under `Dashboard`. A `ChatLayout` holding the conversation sidebar and an `Outlet`. Use the `#[route("...", Component)]` third-argument form already used at `app.rs:52` if a variant name would collide with a `munibot_api` type. |
| 90  | `feat(gui): add conversation sidebar`                 | The conversation list from `list_conversations`, newest first, with new/rename/archive. Selecting one navigates to `ChatConversation`. An empty state that invites a first conversation rather than showing a bare list.                                                                                                                                                                         |
| 91  | `feat(gui): add message list with markdown rendering` | Render assistant messages as markdown via `pulldown-cmark` (pure Rust, compiles to wasm) into `rsx!`. **Code blocks are the centrepiece of the programming use case**: language label, copy button, and syntax highlighting applied client-side by a CDN highlighter, following the `@phosphor-icons` CDN precedent at `app.rs:17` rather than shipping a full grammar set into the wasm bundle. |
| 92  | `feat(gui): add message composer`                     | A growing textarea with enter-to-send and shift-enter for a newline, disabled while a turn is in flight, that keeps its draft when navigating away and back. Pasting a large code block has to feel unremarkable.                                                                                                                                                                                |
| 93  | `feat(gui): add streaming response rendering`         | Consume `ServerEvents<ChatEvent>` and append text deltas to the in-flight assistant message as they arrive. The whole point of the SSE work in phase 11: munibot should feel like he is thinking with you, not batch-processing you.                                                                                                                                                             |
| 94  | `feat(gui): add tool activity display`                | Render `ChatEvent::ToolStarted`/`ToolFinished` as a live, collapsible strip above the reply: tool name, elapsed time, and inspectable result once finished. Richer than Discord's single italic line, because the web has room for it — and it is what makes a twenty-second research turn legible instead of suspicious.                                                                        |
| 95  | `feat(gui): add persona picker`                       | Choose the persona per conversation from `list_personas`, defaulting to the companion. **This replaces automatic routing** for the web: a visible, user-controlled choice is better than an invisible personality switch, especially for a companion.                                                                                                                                            |
| 96  | `feat(gui): add chat error and retry states`          | Match `ChatError` structurally: signed-out prompts a sign-in, a budget refusal explains itself kindly, a provider outage offers a retry that resends the same message without duplicating it. A failed turn must never lose what the user typed.                                                                                                                                                 |

---

## Phase 13 — the companion himself

The phase that earns the title. Everything before this makes munibot work; this makes him _munibot_.

| #   | Commit                                                   | Description                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 97  | `feat(persona): give the companion research tools`       | Grant the companion persona `web_search` and `web_fetch`, and teach `companion.md` to look things up mid-conversation. **The single highest-leverage change in this milestone**: a companion who can answer "wait, is that actually true?" without being swapped for a different persona is the difference between a chat toy and a useful friend. Specialist personas become an opt-in refinement rather than a requirement. |
| 98  | `feat(persona): refine the companion prompt for the web` | Rework `companion.md` for a long-lived, named, one-to-one conversation: no channel context, memory it can reference, and a much longer horizon than a Discord reply. States plainly what it does and does not remember. **This file is the product.** Budget real time for it.                                                                                                                                                |
| 99  | `feat(ai): add crisis recognition`                       | **Moved forward from milestone 5.** A small-model classifier for self-harm, suicidal ideation, abuse disclosure, and acute distress, run on inbound messages for personas with `MemoryPolicy::User`. Returns a severity, not a boolean, and is tuned to over-trigger, because the asymmetry of harm is enormous. A companion people actually confide in needs this _before_ he is public, not in a hardening pass afterwards. |
| 100 | `feat(ai): add crisis response path with resources`      | **Moved forward from milestone 5.** On a positive signal, bypass the normal turn and respond from a reviewed, non-generated template: acknowledge, do not diagnose, do not counsel, surface real region-appropriate resources from a configurable list. **Write this with care and never let a model improvise it.**                                                                                                          |
| 101 | `feat(gui): add memory management panel`                 | See, edit, delete, and wipe everything munibot remembers, plus the opt-in toggle itself. The visible half of the opt-in promise from phase 10 — an opt-in you cannot audit is not really consent.                                                                                                                                                                                                                             |
| 102 | `feat(ai): add conversation title generation`            | Name a conversation from its first exchange with a cheap, hard-capped single-iteration call, leaving the title user-editable. A sidebar full of "new conversation" is unusable within a day.                                                                                                                                                                                                                                  |
| 103 | `feat(gui): add persona catalogue page`                  | A readable listing of configured personas with descriptions, models, and what each can do. Doubles as user-facing documentation for the specialist personas the picker offers.                                                                                                                                                                                                                                                |

---

## Phase 14 — affordability

A public web chat is a far larger cost surface than a Discord bot in a handful of guilds: no invite
gate, no channel gate, and one signed-in stranger can open unlimited conversations. Rate limiting and
spend caps therefore **move forward from milestone 5** to land with the interface that exposes them.

| #   | Commit                                                 | Description                                                                                                                                                                                                                                                                                          |
| --- | ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 104 | `feat(db): add ai rate limit and spend cap tables`     | Migration for `ai_rate_limits` (scope, window start, request count, token count) and `ai_spend_caps` (scope, period, limit micros, current micros, reset at). Scope is a discriminated key covering user, guild, and global, so one mechanism serves every level.                                    |
| 105 | `feat(ai): add rate limiter with sliding window`       | A sliding-window limiter over the database with a small in-memory cache, checked **before** the provider call. Separate limits for requests, tokens, and concurrent turns per scope. Exceeding one returns a friendly lowercase refusal naming when the window resets, per the style in `AGENTS.md`. |
| 106 | `feat(ai): add spend cap enforcement with kill switch` | Track spend per user and globally against configured caps. At 80 percent, warn in the log and the usage panel. At 100 percent, refuse new turns for that scope while letting in-flight ones finish. The global cap is checked first, since it is the last defence against a runaway loop.            |
| 107 | `feat(api): add usage summary server functions`        | `get_my_usage` for the signed-in user and `get_global_usage` for an operator. Aggregate in SQL, not in Rust, or this is slow within a month.                                                                                                                                                         |
| 108 | `feat(gui): add usage and spend panel`                 | What you have spent, against what you are allowed to spend, for the user themselves rather than only an operator. Showing people their own cost is both honest and the cheapest possible abuse deterrent.                                                                                            |

---

## Definition of done

- Signing in, opening the chat page, and talking to munibot works, with replies streaming in.
- A conversation survives a restart with full context, and is still in the sidebar under its own name.
- Long conversations compact themselves instead of erroring on context length.
- Opting in, telling munibot something about yourself, and having him use it next week works end to
  end — and the memory panel shows exactly what he kept.
- Wiping memory removes everything, verifiably, from the same panel.
- Asking the companion a factual question makes him search the web mid-conversation, visibly, without
  changing persona.
- Pasting a stack trace gets a useful answer with readable, copyable code blocks.
- A rate limit or spend cap refuses a turn with a kind, specific message instead of a stack trace.
- A simulated crisis message produces the reviewed template response, never a generated one.
- `MUNIBOT_DISABLE_BOTS=1` still gives a fully working chat page locally.

## Deliberately deferred

- **Automatic persona routing.** Superseded for the web by the explicit picker in commit 95, and made
  largely unnecessary by giving the companion his own tools in commit 97. Revisit only if a real
  surface appears where a user cannot choose.
- **Delegating to specialists.** The companion answers for himself in this milestone. Bringing in a
  specialist — including the twelve-role engineering team ported from `municode` — is milestone 3, and
  it deliberately lands after this milestone's spend caps, since one message can then fan out into
  several nested turns.
- **Twitch AI.** Removed from the plan. Twitch has no message editing, so it needs a completely
  different buffered-chunking renderer, and it is not where the companion gets shown off.
- **Further Discord AI investment.** Phase 8 shipped mention, reply, DM, `/ask`, `/persona`, and
  `/reset`. It stays, unchanged, as a secondary surface.
- **Sign-in beyond Discord OAuth.** The chat is Discord-OAuth-gated for now; additional providers move
  to milestone 5 so the companion is not permanently tied to one account type.
- **Sandbox, repository access, GitHub integration.** Milestones 3 and 4. The coder persona is limited
  to explaining, reviewing, and debugging pasted code until then, which the prompt states plainly.
- **Transcript viewer and guild AI settings.** Administrative rather than companion work; both move to
  milestone 5.
