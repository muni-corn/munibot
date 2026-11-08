# Milestone 01 — the agent loop

**Status:** full recipe. Ready to start.

**Outcome:** in `/chat`, munibot can use tools. Ask him what time it is in Tokyo, and the chat page
shows him calling `current_time` live, then answering correctly. Every turn stays inside a budget,
and a stop button cancels a turn that's running, including any tool it's waiting on.

The loop itself is **rig-agent's**. This milestone adds only what rig-agent lacks, as small
`AgentHook`s: risk tiers, live tool activity, argument validation, model-visible unknown-tool
errors, cost and wall-clock budgets with a final tool-free answer, and retries. History still lives
in the browser, and nothing is saved, so the loop gets proven in isolation before persistence
arrives in milestone 03.

---

## Starting point

Checked against the code after the move to rig 0.46 (see `docs/notes/rig-findings.md`):

- `munibot_ai` is two modules plus a re-export:
  - `config`: `AiConfig { enabled, model: Option<String> }`, where `model` is a rig catalog reference
    such as `anthropic/claude-opus-5`. Load failures are a small `ConfigError`.
  - `conversation`: `Chat { model: DynModel<Completion> }`. `Chat::connect(&str)` goes through
    `Catalog::builtin().connect`. `Chat::reply(Vec<Message>)` streams the text of one model call,
    with the `prompts/conversation.md` preamble and `max_tokens` fixed at 4096.
  - `pub use rig_core as rig`.
- Every model, message, request, stream, usage, and error type is rig's. Tests use rig's
  `MockCompletionModel` (the `test-utils` feature, on the dev-dependency only).
- `munibot_api`:
  - `chat_stream(messages: Vec<ChatMessage>)` is the only chat server function: a `POST` returning
    `ServerEvents<ChatEvent>`.
  - `ChatEvent` is `TextDelta`, `Finished`, or `Failed`.
  - `chat/failure.rs` turns a rig `ProviderError` into a friendly sentence.
- `munibot_gui/src/pages/chat.rs` holds `messages`, `live_reply`, and `failure` signals. It has
  retry, but no stop.
- rig-agent isn't a dependency yet.

## What rig-agent provides, and what this milestone adds

| Need                                 | rig-agent 0.46                                                                  | Added here                                                       |
| ------------------------------------ | ------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| The model → tools → model loop       | `Agent`, `AgentRunner::stream()`                                                | nothing                                                          |
| Tool trait, schemas, results, errors | `rig_core::tool::{Tool, ToolOutput, ToolExecutionError}`, `schema::args_schema` | nothing                                                          |
| Per-call context                     | `ToolContext`, `ContextValue`                                                   | an `Invoker` value: who is acting, and their tier                |
| Parallel tool calls                  | `AgentRunner::tool_concurrency(n)`                                              | nothing; a tool that must not run concurrently serializes itself |
| Cancellation                         | dropping the run's stream drops in-flight tool futures                          | nothing                                                          |
| Full transcript                      | `PromptResponse.messages`                                                       | nothing; milestone 03 stores it                                  |
| Usage and cost per model call        | `CompletionCall.usage`, priced from the catalog                                 | nothing                                                          |
| Narrowing tools per turn             | `RequestPatch::active_tools`                                                    | a hook that narrows to the invoker's tier                        |
| Validating arguments                 | `tool::feedback::arguments_refusal` exists, unused by the loop                  | a hook that applies it before dispatch                           |
| Unknown tool names                   | fail the run by default; `on_invalid_tool_call` can change that                 | a hook returning `unknown_tool_feedback`                         |
| Live tool start and finish           | only after the batch settles                                                    | a hook reporting from `on_dispatch` and `on_outcome`             |
| Budgets                              | `max_turns` only, and running out is an error                                   | a hook for time and cost that forces one last tool-free call     |
| Retrying provider failures           | none                                                                            | retrying a turn that failed before it said anything              |

## Design notes

**Tool calls stay on the server for the length of a turn.** The browser keeps only text. The
transcript in `PromptResponse.messages`, tool calls included, is thrown away when the turn ends,
and only the reply text goes back into the browser's history. Milestone 03 stores the transcript.

**One agent, built once.** `Chat` builds a rig `Agent` at startup with every tool registered. Each
turn narrows what the model is offered through `RequestPatch::active_tools`, so a forbidden tool is
not just refused but never shown. A dispatch hook refuses it again, in case a model names it anyway.

**Hooks talk to the page through a channel.** rig-agent's stream reports tool results only after a
whole batch settles. The activity hook sends live start and finish events into a channel, and
`Chat::reply` merges that channel with rig's stream into one stream of `TurnEvent`s.

**A budget ends in an answer, not an error.** rig-agent treats running out of turns as an error.
The budget hook avoids ever getting there: when the next model call is the last one the budget
allows, or time or money is nearly gone, it patches that call to `ToolChoice::None`, so the model
answers with what it has. Only when time or money is already gone does it stop the run outright.

**Retries are whole turns, before any output.** rig classifies errors and reads `Retry-After`, but
never retries. A turn that fails with `ProviderError::is_retryable()` before anything has reached
the page is simply run again after a backoff. Once text has been shown, a retry would duplicate it,
so a later failure is reported instead.

**Dependencies are added where they're first used**, with `cargo add`, never by hand-editing a
manifest.

---

## Commits

### The loop

#### 1.1 Add rig-agent

`cargo add -p munibot_ai rig-agent`, and `cargo add -p munibot_ai --dev rig-agent -F test-utils`.

```gitcommit
build(ai): add rig-agent

rig-agent is rig's own model and tool loop. using it means munibot only
writes what the loop lacks, as hooks, instead of a loop of its own.
```

#### 1.2 Run replies through a rig agent

`Chat` builds an `Agent` from its `DynModel` with the preamble and `max_tokens`. `reply` splits the
history into prior messages and the final prompt, runs `agent.prompt(prompt).history(prior).stream()`,
and yields the text of `MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text))`.
No behaviour changes: there are no tools yet, and `max_turns` stays 1. The existing tests keep
passing against `MockCompletionModel`.

```gitcommit
refactor(conversation): run replies through a rig agent

moves the chat onto rig-agent's loop before anything depends on it,
so every later commit adds to a loop that already works rather than
replacing one.
```

### Tools and authority

#### 1.3 Add risk tiers

`munibot_ai/src/tools.rs` with `tools/tier.rs`: `RiskTier` is `Safe`, `NetworkRead`, `BotData`,
`Sandbox`, or `Privileged`, ordered by risk. `BotData` is reserved.

```gitcommit
feat(tools): add risk tiers

rig has no notion of who may use which tool; its annotations are a
fixed set of mcp hints. a tool's tier, compared with the tier its
invoker was granted, is what decides whether the model is shown it.
```

#### 1.4 Add the invoker context

`tools/invoker.rs`: `Invoker { user_id: Option<i64>, granted: RiskTier }`, deriving rig's
`ContextValue`, so it travels in every call's `ToolContext` and is never sent to the model.

```gitcommit
feat(tools): carry the invoking person in the tool context

authority comes from the human who sent the message, never from
anything the model asks for. rig's tool context already carries typed
values to every call without showing them to the model.
```

#### 1.5 Add the tool shelf

`tools/shelf.rs`: `ToolShelf` registers rig tools along with a `RiskTier`, adds all of them to an
`AgentBuilder`, and answers `permitted(granted) -> Vec<String>` and `tier_of(name)`. A duplicate
name panics, since it's a programming error.

```gitcommit
feat(tools): add the tool shelf

pairs each rig tool with its risk tier. the agent is built once with
every tool, and the shelf answers which of them a given person may be
offered.
```

### Hooks

#### 1.6 Add turn events

`munibot_ai/src/turn.rs` with `turn/event.rs`. `TurnEvent` is one of:

- `Text(String)`
- `ToolStarted { call_id, name }`
- `ToolFinished { call_id, ok, duration }`
- `Truncated(BudgetLimit)`
- `Finished(rig_agent::PromptResponse)`

Failures are `Err(ProviderError)` items on the stream, not events.

```gitcommit
feat(turn): add turn events

rig's stream reports text as it arrives but tool results only once a
whole batch has settled. the page needs both live, in one stream, so
this is the one event type the chat service adds on top of rig's.
```

#### 1.7 Narrow tools to the invoker's tier

`turn/tier_hook.rs`: an `AgentHook` that reads the `Invoker` and does two things:

- `on_completion_call` patches `active_tools` to the permitted names.
- `on_dispatch` answers `DispatchAction::skip` for anything outside them.

Test with `MockCompletionModel` scripting a call to a forbidden tool.

```gitcommit
feat(turn): narrow tools to the invoker's tier

uses rig's own per-turn tool narrowing, so a forbidden tool is never
offered. the dispatch check is a second line for a model that names
one anyway.
```

#### 1.8 Report tool activity live

`turn/activity_hook.rs`: `on_dispatch` sends `ToolStarted` and records the start time in the run's
scratchpad. `on_outcome` sends `ToolFinished` with the duration and whether the result was a success.
Both send into an unbounded `futures::channel::mpsc` sender the hook holds.

```gitcommit
feat(turn): report tool activity as it happens

a turn that spends ten seconds on a tool looks broken unless the page
can see the tool start. rig's hooks see every call the moment it is
dispatched and settled, so the events come from there.
```

#### 1.9 Validate tool arguments before dispatch

`turn/validation_hook.rs`: `on_dispatch` runs `rig_core::tool::feedback::arguments_refusal` against
the tool's parameters. A refusal answers `DispatchAction::skip(why)`, so the model sees the problem.
A per-run count in the scratchpad stops the run after three refusals.

```gitcommit
feat(turn): validate tool arguments before dispatch

models regularly produce slightly wrong arguments. rig-core already
writes the feedback; this applies it before the tool runs, so the
model can correct itself instead of each tool defending against
malformed input.
```

#### 1.10 Make unknown tool calls model-visible

`turn/invalid_call_hook.rs`: `on_invalid_tool_call` returns
`InvalidToolCallAction::retry(feedback::unknown_tool_feedback(name, permitted))`. The runner sets
`max_invalid_tool_call_retries(3)`.

```gitcommit
feat(turn): let the model recover from naming an unknown tool

rig-agent fails the whole run on an unknown tool by default. telling
the model which tools exist, in rig's own wording, lets it try again.
```

#### 1.11 Add turn budgets

`turn/budget.rs`:

- `TurnBudget { max_turns, max_wall_clock, max_cost_usd }`, all optional. `Default` is 8 model calls,
  60 seconds, and $0.25.
- `BudgetLimit` names which limit was reached.
- `BudgetHook` adds each `CompletionCall`'s `usage.cost` to the scratchpad. Before every call it:
  - patches `ToolChoice::None` when this is the last allowed call, or when the elapsed time or cost
    has passed 80% of its limit, and sends `Truncated`;
  - stops the run outright when time or money is already gone.
- The runner gets `max_turns` from the budget.

Test each limit with `MockCompletionModel` priced through `priced_as`.

```gitcommit
feat(turn): add turn budgets

rig-agent bounds only the number of model calls, and running out is
an error. munibot also bounds time and money, and spends the last
call on a tool-free answer, because a partial answer beats none.
```

#### 1.12 Add budget settings to the ai config

`AiConfig.budget: BudgetConfig { max_turns, max_wall_clock, max_cost_usd }`. Each field is optional
and falls back to `TurnBudget::default()`. Durations parse with `humantime-serde`.

```toml
[ai.budget]
max_turns = 8
max_wall_clock = "60s"
max_cost_usd = 0.25
```

```gitcommit
feat(config): add turn budget settings

operators need to tune how much one message may cost without
recompiling. every field is optional, so existing config files keep
working unchanged.
```

#### 1.13 Retry turns that fail before saying anything

- `cargo add -p munibot_ai backon`.
- `turn/retry.rs`: when a turn's stream fails with `is_retryable()` before any `Text` or
  `ToolStarted` event, run it again. Use `backon::ExponentialBuilder`'s delays (three attempts), or
  the provider's `ErrorReport::retry_after`, whichever is longer.
- Test with a scripted 503 followed by success.

```gitcommit
feat(turn): retry turns that fail before saying anything

rig classifies provider errors and reads retry-after, but never
retries. retrying is limited to turns that haven't shown anything yet,
because replaying text the reader has already seen would duplicate it.
```

### Built-in tools and wiring

#### 1.14 Add the current time tool

- `cargo add -p munibot_ai chrono chrono-tz`.
- `tools/builtin/current_time.rs`: a rig `Tool` taking an optional IANA `timezone` and returning an
  RFC 3339 timestamp plus a readable one. Its parameters come from `tool::schema::args_schema`.
  Registered on the shelf at `RiskTier::Safe`.

```gitcommit
feat(tools): add the current time tool

models have no clock. it is also the smallest possible real tool,
which makes it the right one to prove the whole path end to end.
```

#### 1.15 Give the chat its tools and budget

- `Chat` builds its agent with the shelf and the four hooks.
- `Chat::reply(history, invoker)` returns `BoxStream<'static, Result<TurnEvent, ProviderError>>`,
  merging rig's stream with the activity channel.
- `munibot/src/ai.rs::build` passes the budget config through.
- `chat_stream` builds an `Invoker` for the signed-in user (`granted: RiskTier::Safe`), and
  `ChatEvent::stream_from_reply` maps `TurnEvent::Text`, ignoring the rest for now.

```gitcommit
feat: give the chat its tools and turn budget

the chat now runs real agent turns with the built-in tools, inside a
budget. the reply's signature changes across the crates that call it,
so they move together; tool activity reaches the page next.
```

#### 1.16 Tell munibot what his tools are for

`prompts/conversation.md` stops claiming he has no tools. It says he can check the time and should
use tools when they help, and still says plainly that he can't browse, remember past chats, or run
code. A `conversation.rs` test asserts the prompt doesn't say "no tools".

```gitcommit
feat(ai): tell munibot what his tools are for

the prompt still said he had no tools at all, which would make him
refuse to check the time while holding a tool that does exactly that.
```

#### 1.17 Add tool activity to chat events

`munibot_api/src/chat/event.rs`:

- `ChatEvent::ToolStarted { id, name }`.
- `ChatEvent::ToolFinished { id, ok, duration_ms }`.
- `ChatEvent::Truncated(String)`, a friendly explanation of which limit was reached.

The `TurnEvent` to `ChatEvent` mapping is a pure function and is tested directly.

```gitcommit
feat(event): add tool activity to chat events

the page needs to see tools start and finish while a turn runs, and to
learn when a reply was cut short by a budget rather than finished
naturally.
```

#### 1.18 Show tool activity while munibot works

`munibot_gui/src/components/chat/tool_activity.rs`: a compact strip above the live reply. Each tool
shows its name, a spinner while running, then a check or cross with its duration. When the turn
finishes, the strip stays attached to that reply in page state; it's never sent back to the server.
A truncated reply shows a small note saying why.

```gitcommit
feat(gui): show tool activity while munibot works

makes a turn's tool work visible, and keeps it next to the answer it
produced.
```

#### 1.19 Add a stop button

While a reply is running, the composer's send button becomes a stop button. Pressing it drops the
event stream. That closes the server-sent events connection, the server drops rig's run stream, and
rig drops any tool still running. Partial text is kept as the reply, with a "stopped" note.

```gitcommit
feat(gui): add a stop button

a wrong or runaway reply should be stoppable from the page. dropping
the stream is enough: rig cancels a run, tools included, when its
stream is dropped.
```

#### 1.20 Document tools and budgets

- `docs/chat.md`: tools exist, which tools there are, the `[ai.budget]` table, and the stop button.
  Keep "no saved conversations, no rate limits".
- `docs/notes/rig-findings.md`: record anything about rig-agent's hooks that surprised you while
  building this milestone.

```gitcommit
docs: document chat tools and turn budgets

the chat guide still said munibot has no tools.
```

---

## Definition of done

- Asking "what time is it in Tokyo?" in `/chat` shows `current_time` running and then finishing in
  the activity strip, followed by a correct answer.
- With `max_turns = 1`, a turn that wants a tool still produces an answer, plus a note that it was
  cut short.
- Pressing stop during a turn ends it within a second, including when a tool is mid-call.
- A scripted 503 followed by success makes the turn succeed, with no duplicated text.
- A scripted call to a tool outside the invoker's tier is never dispatched.
- Every hook is tested against `MockCompletionModel`, or with rig-agent's `HookTestHarness`.
  `devenv test` passes with no network.

## Deliberately deferred

- **Web tools.** `web_search` and `web_fetch` are milestone 02, along with the untrusted-content
  wrapper they need.
- **Saving transcripts.** `PromptResponse.messages` has nowhere to go until milestone 03.
- **Serial tools.** rig-agent treats every call alike. A tool that must not run alongside itself, like
  a shell, serializes itself internally, which is what rig's own docs recommend.
- **Per-user limits.** A budget bounds one turn, but nothing bounds how many turns a person runs.
  That's milestone 04, and it's why `docs/chat.md` keeps warning that anyone signed in can spend the
  provider key.
- **Structured handoffs.** Only the autonomous pipeline needs them, in milestone 10.
