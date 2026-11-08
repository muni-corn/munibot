# Milestone 03 — saved conversations

**Status:** full recipe. Start after [milestone 02](milestone-02-research.md). Do
[loose ends](../loose-ends.md) commit 1.6 (the test fixture) first, because every integration test
here depends on it.

**Outcome:** `/chat` becomes a real conversation list. Conversations survive a reload, a restart, and
a different device. Each one has a title and a sidebar entry, and can be renamed, archived, or
deleted. munibot remembers what his tools returned earlier in a conversation, and a long
conversation compacts itself instead of failing on context length. Every turn's cost and every tool
call is recorded.

That recording is what milestone 04's spend caps read, and the stored transcript is what every later
milestone builds on.

---

## Starting point

This milestone assumes milestones 01 and 02 are done:

- `Chat::reply(history, invoker)` runs a rig-agent turn over a history the browser sends in full.
- rig's `PromptResponse.messages` holds every message a turn appended, including tool calls and
  their results, and that transcript is thrown away.
- Every message in play is a rig `Message`.
- `chat_stream(messages)` validates the browser's history against `validate_conversation`: 200
  messages and 100,000 characters (`munibot_api/src/chat/validate.rs`).
- `munibot_ai` does not depend on `munibot_core`.

From the code as it stands:

- Migrations are embedded and applied at boot (`munibot/src/main.rs:53`). The newest migration is
  `2026-09-01-110000-0000_add_email_signin_tokens`.
- `users.id` is a `BIGINT`, and `linked_accounts.user_id` has **no** `ON DELETE CASCADE`. See loose
  ends commit 2.1, which has to land before "deleting a user deletes their conversations" is
  actually true.
- `TestDb` lives in `munibot_core/tests/common/mod.rs`. Only `munibot_core`'s own integration tests
  can use it.
- `munibot_core/src/db/operations.rs` is about 760 lines, so new operations go in a submodule.
- The workspace `diesel` has `chrono`, `mysql`, and `uuid` features, but not `serde_json`.
- The chat route is `#[route("/chat")] Chat {}` under `MainLayout` and `HomeLayout`, and outside
  `Dashboard` (`munibot_gui/src/app/route.rs`).

## Design notes

**The server owns history now.** The browser sends only the new message. The full transcript,
including tool calls and results, lives in `ai_messages`, so munibot can refer back to a page he read
three turns ago.

**Rows hold rig's own message JSON.** `ai_messages.content` is a rig `Message`, serialized by rig's
serde. There's no munibot copy of the message type to keep in sync. The catch is that rig's
serialized form has changed between releases: 0.45 can't read tool-call turns that 0.41 stored. So
every row records the rig version that wrote it (`rig_format`), and a fixture test loads a
committed sample of stored messages, so a rig upgrade that breaks old rows fails the test suite
instead of breaking conversations.

**What rig-memory provides, and what it doesn't.** `rig-memory` is built for a single local agent:
an append-only log per conversation, no owners, no listing, and whole-history loads. That doesn't
fit a multi-user server on MySQL, so storage, ownership, and listing stay munibot's own. Its
history-shaping pieces work on plain `&[Arc<Message>]` and are reused directly:

| Need                                          | Reused from rig                                               |
| --------------------------------------------- | ------------------------------------------------------------- |
| Keeping the newest messages within a budget   | `rig_memory::recent::{view, TokenBudget}`                     |
| Never splitting a tool call from its result   | `view` applies `rig_core::conversation::pairing::safe_start`  |
| Estimating tokens                             | `rig_memory::HeuristicTokenCounter`                           |
| Choosing where to cut for a summary           | `rig_memory::compaction::CompactionPolicy::cut`               |
| Building the summary request                  | `CompactionPolicy::plan`, with munibot's own `SummaryPrompts` |
| Rejecting a truncated or tool-calling summary | `Summarizer::summary_text`                                    |

**Conversations are created lazily.** Opening `/chat` doesn't create anything. The first message
does, and the stream's first event carries the new conversation's id so the page can move to
`/chat/:id`. A sidebar full of empty conversations is never possible.

**Compaction summarizes and keeps the messages.** Once a conversation's history exceeds its context
budget, the oldest messages are summarized, and rig-memory's `SummaryState` is stored in
`ai_conversations.summary_state`. `summary_through_seq` records where the summary ends. Context assembly skips anything at or before
that point and prepends the summary instead. The messages themselves stay, so the conversation view
still shows everything that was said.

**One turn per conversation at a time.** A second send while a turn is running would interleave two
transcripts. An in-memory lock keyed by conversation id refuses it with a friendly error. munibot
runs as a single process, so in-memory is enough.

**What gets saved when a turn doesn't finish:**

| Ending    | What's saved                                                                  |
| --------- | ----------------------------------------------------------------------------- |
| Completed | the user's message and the turn's full transcript                             |
| Truncated | the same, plus the truncation note                                            |
| Stopped   | the user's message, plus partial text as an assistant message, no tool blocks |
| Failed    | the user's message only, so retry can answer it                               |

Usage and tool-call audit rows are written in **every** case, because a turn that failed on its
fifth iteration still cost money.

**Spend rows outlive what they describe.** `ai_usage` foreign keys use `ON DELETE SET NULL`, so
deleting a conversation or a user doesn't erase global spend history. `ai_tool_calls` cascades with
its conversation, since it's an audit of content the user chose to delete.

**A utility model handles housekeeping.** Titles and summaries are small, single, tool-free calls.
`[ai] utility_model` lets a cheap model do them. It defaults to the chat model.

---

## Schema

```
ai_conversations (id BIGINT PK, owner_user_id BIGINT NOT NULL -> users ON DELETE CASCADE,
                  title VARCHAR(200) NULL, summary_state JSON NULL, summary_through_seq INT NULL,
                  created_at, last_active_at, archived_at NULL,
                  INDEX (owner_user_id, archived_at, last_active_at))

ai_messages      (id BIGINT PK, conversation_id -> ai_conversations ON DELETE CASCADE,
                  seq INT NOT NULL, role VARCHAR(16), content JSON, rig_format VARCHAR(16),
                  created_at,
                  UNIQUE (conversation_id, seq))

ai_usage         (id BIGINT PK, user_id NULL -> users ON DELETE SET NULL,
                  conversation_id NULL -> ai_conversations ON DELETE SET NULL,
                  provider VARCHAR(32), model VARCHAR(128),
                  usage JSON, model_cost_micros, tool_cost_micros, outcome VARCHAR(16),
                  created_at, INDEX (user_id, created_at), INDEX (created_at))

ai_tool_calls    (id BIGINT PK, conversation_id NULL -> ai_conversations ON DELETE CASCADE,
                  user_id NULL -> users ON DELETE SET NULL, tool_name VARCHAR(64),
                  input JSON, output TEXT, ok BOOL, duration_ms INT, created_at,
                  INDEX (conversation_id, created_at))
```

`ai_usage.usage` is rig's `Usage`, stored whole, so new counters rig adds are kept without a
migration. The two cost columns are integers, converted from rig's dollar figures at the boundary,
because spend caps sum them in SQL. `role` duplicates what's inside `content`, so the conversation
list can be queried without parsing JSON. `ai_tool_calls.input` and `output` are truncated to
16 KB before they're written.

---

## Commits

### Shared test fixture

#### 3.1 Share the test database fixture

- Move `TestDb` from `munibot_core/tests/common/mod.rs` to `munibot_core/src/test_util.rs`, behind a
  `test-util` feature.
- `munibot_core`'s own tests enable it through a `[dev-dependencies]` self-reference.
- Other crates add `munibot_core` with `features = ["test-util"]` as a dev-dependency, using
  `cargo add --dev`.

```gitcommit
refactor(core): share the test database fixture behind a feature

the ai crate's stores need real database tests too. an integration
test's common module can't be shared across crates, so the fixture
moves into the library behind a feature that only tests enable.
```

### Database

#### 3.2 Add conversation and message tables

- Add a migration for `ai_conversations` and `ai_messages`, as in the schema above.
- Enable diesel's `serde_json` feature for `munibot_core` (`cargo add -p munibot_core diesel -F serde_json`).
- Regenerate `schema.rs`.
- In `models.rs`, add `AiConversation`, `NewAiConversation`, `AiMessageRow`, and `NewAiMessage`. Each
  gets `#[diesel(check_for_backend(diesel::mysql::Mysql))]`.

```gitcommit
feat(db): add ai conversation and message tables

stores a person's conversations and every message in them, tool calls
included, so history survives a reload and munibot can refer back to
what his tools found. both tables cascade from their owner.
```

#### 3.3 Add usage and tool call tables

Add a migration for `ai_usage` and `ai_tool_calls`, plus their schema and models.

```gitcommit
feat(db): add ai usage and tool call tables

every turn's cost and every tool call get a row. usage rows survive
deleting their conversation or user, because global spend has to
stay accurate after someone tidies up.
```

#### 3.4 Add conversation operations

`munibot_core/src/db/operations/ai.rs`:

- `create_conversation`
- `get_owned_conversation(id, owner)`, which returns `None` for someone else's conversation
- `list_conversations(owner, include_archived)`
- `append_messages(id, Vec<NewAiMessage>)`, which assigns `seq` inside a transaction
- `load_messages(id, after_seq)`
- `set_title`
- `set_summary(id, summary, through_seq)`
- `archive`
- `delete_conversation`
- `touch(id)`

Integration tests use `TestDb`.

```gitcommit
feat(db): add ai conversation operations

ownership is part of every read, so no caller can fetch someone else's
conversation by guessing an id. sequence numbers are assigned in a
transaction so concurrent appends can never collide.
```

#### 3.5 Add usage and tool call operations

`insert_usage`, `insert_tool_call`, and `sum_cost_since(user_id: Option, since)`. The last one is
unused until milestone 04, but tested here because it reads this milestone's table.

```gitcommit
feat(db): add ai usage and tool call operations

writes the accounting rows and adds the one aggregate query spend caps
will need, done in sql rather than by loading rows into memory.
```

### Memory module

#### 3.6 Depend on munibot_core and rig-memory

`cargo add -p munibot_ai munibot_core --path munibot_core`, the dev-dependency with `test-util`, and
`cargo add -p munibot_ai rig-memory`.

```gitcommit
build(ai): depend on munibot_core and rig-memory

the ai crate's stores persist through munibot_core's database layer,
and rig-memory supplies the context windowing, token estimates, and
compaction planning those stores feed. munibot_core knows nothing
about ai, so this adds no cycle.
```

#### 3.7 Add conversation types

`munibot_ai/src/memory.rs`, with `memory/` and `memory/types.rs`:

- `ConversationId(i64)`.
- `Conversation { id, owner, title, summary: Option<rig_memory::compaction::SummaryState>,
summary_through_seq, last_active_at }`.
- `ConversationSummary` for listings.
- `StoredMessage { seq, message: rig_core::message::Message }`.
- `TurnError`: `NotFound`, `Busy`, `Store(String)`, or `Provider(ProviderError)`. The chat service
  returns it once turns touch storage. rig has no error for "not yours" or "already answering".
- `TurnEvent::Started { conversation }`, which the chat service sends first.

```gitcommit
feat(memory): add conversation types

the vocabulary every store and the chat service share. messages are
rig's own; only what rig has no notion of is added here: who owns a
conversation, its title, and where its summary ends. kept separate
from the database models so nothing above the store depends on diesel.
```

#### 3.8 Add the session store

- `memory/session.rs`: an object-safe `SessionStore` trait with:
  - `create(owner)`
  - `get(id, owner)`
  - `append(id, Vec<Message>) -> Vec<u32>`
  - `messages(id, after_seq)`
  - `set_summary`
  - `set_title`
  - `touch`
- `InMemorySessionStore` for tests.

```gitcommit
feat(memory): add the session store

one conversation's history behind a trait, so the chat service and its
tests never depend on a database. the in-memory store is what nearly
every later test runs against.
```

#### 3.9 Add the conversation directory

- `memory/directory.rs`: a `ConversationDirectory` trait with `list(owner, include_archived)`,
  `rename`, `archive`, and `delete`.
- Keep it separate from `SessionStore`: listing someone's conversations is a different question from
  reading one of them.
- Include an in-memory implementation.

```gitcommit
feat(memory): add the conversation directory

listing and managing a person's conversations is a different job from
reading one conversation's history, and folding both into one trait
would force every store to implement both.
```

#### 3.10 Add diesel stores

`memory/diesel.rs`: `DieselSessionStore` and `DieselConversationDirectory` over `DbPool`. Rows hold
rig's own serialization of each `Message`, stamped with `rig_format`. Integration tests use the
shared `TestDb`.

Add `memory/fixtures/stored_messages.json`, a committed sample of rig-serialized messages covering
text, a tool call, a tool result, and reasoning, plus a test that deserializes it. That test is what
fails when a rig upgrade can no longer read rows already in the database.

```gitcommit
feat(memory): add diesel-backed conversation stores

the production implementations of both traits, storing rig's own
message json. rig's serialized form has changed between releases, so a
committed fixture of stored messages turns an incompatible rig upgrade
into a failing test rather than unreadable conversations.
```

#### 3.11 Add context assembly

- `memory/context.rs`: `assemble_context(&Conversation, Vec<StoredMessage>, max_tokens) -> Vec<Message>`.
- It skips messages at or before `summary_through_seq`, then keeps the newest messages that fit
  through `rig_memory::recent::view` with a `TokenBudget` of `HeuristicTokenCounter`. `view` already
  moves the start past any tool result whose call was cut off.
- The summary is prepended as a user-role message. The newest message always survives, even with a
  zero budget.

```gitcommit
feat(memory): assemble context within a token budget

decides which part of a long conversation the model actually sees.
rig-memory already windows a history by token budget without ever
separating a tool call from its result, so this only adds what it
can't know: where munibot's stored summary ends.
```

### Accounting

#### 3.12 Add usage and audit recorders

`munibot_ai/src/accounting.rs`:

- `UsageRecord { user, conversation, provider, model, usage: rig_core::completion::Usage, tool_cost, outcome }`,
  and a `UsageRecorder` trait. Dollar figures become micro-dollar columns here, and nowhere else.
- `ToolCallRecord`, and a `ToolAuditor` trait.
- Diesel implementations, plus in-memory fakes that capture records for tests.

```gitcommit
feat(ai): add usage recording and tool call auditing

records what each turn cost and what each tool did. the traits keep
the chat service testable, and the rows are what spend caps and
transcripts will read.
```

#### 3.13 Audit every tool call

`turn/audit_hook.rs`: an `AgentHook` that takes each call's arguments from `on_dispatch` and its
result from `on_outcome`, and writes a `ToolCallRecord` through the `ToolAuditor`.

```gitcommit
feat(turn): audit every tool call

the audit trail needs what each tool was asked to do and what it
returned. rig's dispatch and outcome hooks see both for every call,
so auditing is one more hook rather than a change to the events the
page receives.
```

### Configuration

#### 3.14 Add context and utility model settings

`AiConfig.context_tokens` (default 100,000), `AiConfig.utility_model: Option<String>` (a rig catalog
reference, connected at startup like `model`), and `AiConfig.max_message_chars` (default 32,000).

```gitcommit
feat(config): add context, utility model, and message length settings

lets an operator size the context window, pick a cheap model for
titles and summaries, and cap how long one message may be now that
the whole history is no longer sent from the browser.
```

### Behaviour

#### 3.15 Run turns against saved conversations

`Chat::turn(conversation: Option<ConversationId>, text, invoker) -> BoxStream<'static, Result<TurnEvent, TurnError>>`:

1. Take the per-conversation lock.
2. Create the conversation if needed, or load it with an ownership check.
3. Append the user message, then assemble context and run the agent with the audit hook.
4. Persist `PromptResponse.messages` according to the "what gets saved" table above.
5. Write a usage row from the response's `Usage`, then touch the conversation.

The stream yields `TurnEvent::Started { conversation }` first. A busy conversation fails with
`TurnError::Busy`.

`Chat::connect` now also takes the stores and recorders. `munibot/src/ai.rs::build` builds the diesel ones
from the pool, and `main.rs` passes the pool in. `Chat::reply` stays as it is for now.

```gitcommit
feat(ai): run turns against saved conversations

the chat service loads history itself, saves the full transcript,
and records what the turn cost. stopped and failed turns are handled
explicitly so a retry never duplicates a message and a stop never
loses what was already on screen.
```

#### 3.16 Compact long conversations

`memory/compaction.rs`, before assembling context:

1. If the unsummarized history is over budget, ask `CompactionPolicy::cut` where to cut. It only cuts
   before a message that can start a context, so no tool call loses its result.
2. Build the request with `CompactionPolicy::plan`, using munibot's own `SummaryPrompts` (rig's
   defaults are written for a coding agent's checkpoints, not a conversation).
3. Send it to the utility model, check the reply with `Summarizer::summary_text`, and store the
   resulting `SummaryState` with `set_summary`.

Tested with `MockCompletionModel` and the in-memory store.

```gitcommit
feat(memory): compact long conversations

a conversation that outgrows the context window is summarized from
the oldest end instead of failing. rig-memory already picks safe cut
points, builds incremental summary requests, and rejects truncated
summaries; munibot supplies the prompts and keeps every message for
display.
```

#### 3.17 Generate conversation titles

After a conversation's first completed turn, make one tool-free utility-model call, capped at 30
tokens, to set a title of eight words or fewer. A title the user already set is never overwritten.

```gitcommit
feat(ai): generate conversation titles

a sidebar full of "new conversation" is useless within a day. the
call is tiny and only happens once, and a rename always wins.
```

### API

#### 3.18 Add conversation wire types

`munibot_api/src/chat/`:

- `ConversationSummary`.
- `ConversationView { id, title, messages: Vec<MessageView> }`.
- `MessageView { role, text, activity: Vec<ToolActivityView>, stopped }`, built server-side from the
  stored transcript. Tool-result messages are folded into the preceding assistant message's activity
  rather than shown as user bubbles.
- `ChatEvent::Started { conversation_id }`.
- `ChatError::{NotFound, Busy, MessageTooLong}`.

```gitcommit
feat(chat): add conversation wire types

the page shows a saved conversation the way it was seen live: replies
with their tool activity attached, and no tool results masquerading as
messages from the user.
```

#### 3.19 Add conversation server functions

`list_conversations`, `get_conversation(id)`, `rename_conversation`, `archive_conversation`, and
`delete_conversation`. Each resolves the caller from `AuthSession` and goes through the
ownership-checked operations, so another person's conversation is indistinguishable from one that
doesn't exist.

```gitcommit
feat(api): add conversation server functions

lists and manages a person's own conversations. someone else's
conversation reads as not found rather than forbidden, so ids reveal
nothing.
```

#### 3.20 Add message sending

`send_message(conversation_id: Option<i64>, text)` and `retry_reply(conversation_id)` are both
`POST`s returning `ServerEvents<ChatEvent>`, the same pattern `chat_stream` already uses. Text over
`max_message_chars` is refused. Retrying a conversation whose last message isn't from the user is
refused.

```gitcommit
feat(api): add sending messages to saved conversations

the browser now sends one message, not the whole history. retry
answers the existing last message instead of sending it again, so a
failed turn never leaves a duplicate behind.
```

### GUI

#### 3.21 Add the chat layout and routes

`Route::NewChat` at `/chat` and `Route::ChatConversation { conversation_id }` at `/chat/:conversation_id`,
both under a new `ChatLayout` (sidebar plus `Outlet`) inside `HomeLayout`.

```gitcommit
feat(gui): add chat layout and conversation routes

every conversation gets its own url, so a reload or a shared link
lands back in the right place.
```

#### 3.22 Add the conversation sidebar

Lists conversations newest first, with a "new conversation" button and inline rename, archive, and
delete. Delete asks for confirmation. When the list is empty, it invites a first conversation.

```gitcommit
feat(gui): add the conversation sidebar

the list of everything you have talked to munibot about, with the
management actions a list needs.
```

#### 3.23 Continue saved conversations

- The chat page loads `get_conversation` and sends with `send_message`. On `Started`, it replaces
  the route with `/chat/:id` without remounting.
- Retry uses `retry_reply`.
- Restored replies show their tool activity, collapsed.
- Remove the "this conversation isn't saved" banner.

```gitcommit
feat(gui): continue saved conversations

the page now reads history from the server and sends only what you
type. moving to the new conversation's url happens without a reload,
so a reply streams uninterrupted through it.
```

#### 3.24 Remove the stateless chat path

Delete `chat_stream`, `validate_conversation` and its constants, and `Chat::reply`. Nothing calls
them anymore.

```gitcommit
refactor!: remove the stateless chat path

every caller now uses saved conversations. keeping the old path would
leave a second way into the model with none of the new accounting.

BREAKING CHANGE: the /api/ai/chat/stream server function is removed in
favour of send_message and retry_reply.
```

#### 3.25 Document saved conversations

- `docs/chat.md`: conversations are saved, what is stored and for how long, compaction, titles, the
  new config keys, and how deletion works. Keep the warning that there are no rate limits yet.
- `docs/gui.md`: the chat layout and routes.

```gitcommit
docs: document saved conversations

the chat guide still described a page that saved nothing. it now says
plainly what is kept, how to delete it, and what still isn't limited.
```

---

## Definition of done

- A conversation survives a page reload and a server restart, with full context. munibot can quote a
  search result from three turns earlier without searching again.
- The sidebar lists conversations under generated titles, and rename, archive, and delete all work.
  Deleting removes the messages and tool calls but keeps the usage rows.
- Another account requesting your conversation id gets "not found".
- A conversation pushed past `context_tokens` keeps working and gains a summary, and its full message
  list is still visible.
- Stopping a turn keeps the partial reply, and retrying a failed turn produces exactly one user
  message.
- Every turn writes an `ai_usage` row, including failed and stopped turns, and every tool call writes
  an `ai_tool_calls` row.
- `devenv test` passes. Store tests run against the shared `TestDb`.

## Deliberately deferred

- **Rate limits and spend caps.** Usage is now recorded. Milestone 04 reads it.
- **Personas.** Conversations have no persona column yet. Milestone 05 adds one, when there's more
  than one persona to record.
- **Memory across conversations.** Milestone 06.
- **Discord-scoped conversations.** `owner_user_id` is required here. Milestone 12 adds a scope for
  channel conversations, and decides what owning one means there.
- **Export.** Downloading your own conversations is worth adding once people have conversations
  worth keeping.
