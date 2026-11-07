# Milestone 01 — the agent loop

**Status:** full recipe. Ready to start.

**Outcome:** in `/chat`, munibot can use tools. Ask him what time it is in Tokyo, and the chat page
shows him calling `current_time` live, then answering correctly. Every turn stays inside a budget,
and a stop button cancels a turn that's running, including any tool it's waiting on.

This milestone builds the machinery every later milestone depends on: tool-capable types, tool calls
through rig, a mock provider that can script tool calls, retries, pricing, the tool system, and the
agent loop. It keeps the current page's shape on purpose. History still lives in the browser, and
nothing is saved, so the loop gets proven in isolation before persistence arrives in milestone 03.

---

## Starting point

Everything below is checked against the code as it stands at `9b4a38e1`.

- `munibot_ai/src/types/`:
  - `Role` is `User` or `Assistant` only (`content.rs:10`).
  - `ContentBlock` is `Text` or `Thinking` only (`content.rs:27`).
  - `CompletionRequest` has no `tools` field (`completion.rs:12`).
  - `StopReason` has no `ToolUse` (`completion.rs:52`).
  - `StreamEvent` is `TextDelta`, `ThinkingDelta`, `Usage`, or `Done` (`stream.rs:12`).
  - `AiError` is `Provider`, `Rejected`, `RateLimited`, `Config`, or `Other`, with `is_transient()`
    (`error.rs:10`).
  - There's no `Cost` and no tool schema type.
- `munibot_ai/src/provider/`:
  - `Provider` (`provider.rs:31`): its default `stream` replays `complete`.
  - `MockProvider` (`mock.rs:19`): scripts text and errors only.
  - `RigProvider` (`rig/adapter.rs:28`):
    - sends `tools: vec![]`;
    - reports every stop as `EndTurn`;
    - drops `top_p` and `thinking_budget`.
  - `convert.rs` drops rig tool calls and images.
  - `ProviderResolver` caches by the full `provider:model` string.
  - There's no retry layer and no pricing.
- `munibot_ai/src/conversation.rs`:
  - `Chat::reply(Vec<Message>)` makes one provider call and returns a stream of text chunks.
  - It sends the `prompts/conversation.md` system prompt, and `max_tokens` is fixed at 4096.
  - That prompt tells the model it has no tools.
- `munibot_ai/src/config.rs`: `AiConfig { enabled, model }`, parsed from the `[ai]` table of the
  shared config file.
- `munibot_api`:
  - `chat_stream(messages: Vec<ChatMessage>)` is the only chat server function. It's a `POST` that
    returns `ServerEvents<ChatEvent>` (`server_fns/chat/stream.rs:15`).
  - `ChatEvent` is `TextDelta`, `Finished`, or `Failed` (`chat/event.rs:5`).
- `munibot_gui/src/pages/chat.rs` holds `messages`, `live_reply`, and `failure` signals. It has
  retry, but no stop.
- `munibot_ai`'s own dependencies are `async-stream`, `async-trait`, `futures`, `rig-core`, `serde`,
  `thiserror`, and `toml`. It has no `tracing`, `serde_json`, `schemars`, `jsonschema`,
  `tokio-util`, or `backon`.

## Design notes

**Tool calls stay on the server for the length of a turn.** The browser keeps only text. Within one
turn, the loop's assistant tool-use messages and tool-result messages live in the harness and are
thrown away when the turn ends. Only the reply text goes back into the browser's history. The model
loses the raw tool output on later turns, but keeps whatever it said about it. That's an honest
limit of a stateless page, and milestone 03 removes it by storing the full transcript.

**Tool calls arrive whole, not as deltas.** rig 0.41's streaming yields a complete
`StreamedAssistantContent::ToolCall` once the arguments are fully known, alongside optional partial
`ToolCallDelta`s. The harness can't act on half an argument object, so `StreamEvent::ToolUse` carries
the complete call and the adapter ignores deltas. The old plan's `ToolUseStart`/`ToolUseDelta`/
`ToolUseEnd` trio was more surface than any consumer needed.

**Tool results ride in a user-role message.** That's how both Anthropic and rig model them, so `Role`
gains no `Tool` variant.

**Tool call identity is lossless.** rig's `ToolCall` and `ToolResult` carry both an `id` and an
optional provider-specific `call_id`, and some APIs need both echoed back. Our blocks store both
rather than choosing one, so nothing depends on guessing which one a provider correlates by.

**The loop is streaming-first.** `Harness::run_turn_streamed` is the one implementation.
`run_turn` folds its events into a `TurnOutcome`. Tool start and finish events are emitted from the
very first version of the loop, so no consumer is ever written against events that don't exist yet.

**Stop reason is inferred.** rig exposes no normalized finish reason
(`docs/notes/rig-provider-findings.md`). The adapter reports `ToolUse` when a response contains a
tool call, and `EndTurn` otherwise.

**Dependencies are added where they're first used**, with `cargo add -p munibot_ai <crate>`, never by
hand-editing a manifest.

---

## Commits

### Types

#### 1.1 Add tool use and tool result content blocks

- Add `serde_json` to `munibot_ai` (`cargo add -p munibot_ai serde_json`).
- `ContentBlock::ToolUse { id, provider_call_id: Option<String>, name, arguments: serde_json::Value }`
  and `ContentBlock::ToolResult { id, provider_call_id: Option<String>, content: String, is_error: bool }`.
  Use the existing internal `type` tag, so the variants serialize as `tool_use` and `tool_result`.
- `ContentBlock::as_tool_use()` returns a borrowed view.
- `Message::tool_results(Vec<ContentBlock>)` builds a user-role message.
- `Message::text()` keeps skipping everything that isn't text.
- Add serde roundtrip tests for both variants.

```gitcommit
feat(types): add tool use and tool result content blocks

the agent loop needs to carry a model's tool calls and their results
through history. both blocks keep rig's id and its optional
provider-specific call id, since some providers correlate results by
one and some by the other, and echoing both back is the only choice
that is never wrong.
```

#### 1.2 Add a tool schema type

- Add `schemars` (`cargo add -p munibot_ai schemars`).
- `types/tool.rs`: `ToolSchema { name, description, input_schema: serde_json::Value }`, plus
  `ToolSchema::from_type::<T: JsonSchema>(name, description)` so a tool derives its schema from its
  own input struct.
- Test that a derived schema has the expected `properties` and `required`.

```gitcommit
feat(types): add a tool schema type

describes a tool to a model in provider-neutral terms. deriving the
input schema from the tool's own argument struct keeps the schema and
the deserialization target from ever drifting apart.
```

#### 1.3 Add tools and tool choice to completion requests

- `CompletionRequest.tools: Vec<ToolSchema>`, with `#[serde(default)]`, and `with_tools`.
- `ToolChoice` is `Auto`, `None`, or `Required`. Add `CompletionRequest.tool_choice: ToolChoice`
  (default `Auto`) and `with_tool_choice`.
- Add `StopReason::ToolUse`.
- `CompletionResponse::tool_uses()` returns every tool-use block in order.

```gitcommit
feat(types): add tools and tool choice to completion requests

lets a request offer tools and lets a response say it stopped to call
them. tool choice exists so the loop can make one final, tool-free
call when a budget runs out, rather than ending a turn with no answer.
```

#### 1.4 Add a tool use stream event

- `StreamEvent::ToolUse { id, provider_call_id, name, arguments }`: one complete call, never a
  partial one.
- Add a roundtrip test.

```gitcommit
feat(types): add a tool use stream event

a streamed reply has to surface the tool calls it ends with. calls are
emitted whole because nothing downstream can act on half an argument
object, and rig already yields them whole.
```

#### 1.5 Add a cost type

- `types/cost.rs`: `Cost(i64)` in micro-dollars, with `Add`, `AddAssign`, `Sum`, `Ord`,
  `Cost::from_usd(f64)`, and a `Display` that prints dollars to four places.
- Add tests for summing and formatting.

```gitcommit
feat(types): add a cost type

turn budgets and, later, spend caps sum and compare costs constantly.
integer micro-dollars keep that exact, where floating point would
drift across thousands of additions.
```

#### 1.6 Add cancelled and tool errors

- `AiError::Cancelled` reads "the reply was stopped".
- `AiError::Tool(String)` is for tool failures that end the turn rather than being reported back to
  the model.
- Neither is transient.

```gitcommit
feat(types): add cancelled and tool error variants

the loop needs to say a turn was stopped on purpose, which no existing
variant fits. it also needs to report the rare tool failure that
should end a turn rather than be handed back to the model to work
around.
```

### Provider

#### 1.7 Convert tools to and from rig

`provider/rig/convert.rs`:

- `ToolSchema` converts to rig's `ToolDefinition`.
- Our `ToolUse` and rig's `AssistantContent::ToolCall` convert both ways, keeping `id` and `call_id`.
- `ToolResult` converts to `UserContent::ToolResult`.
- `from_streamed_content` maps `StreamedAssistantContent::ToolCall` to `StreamEvent::ToolUse`. It
  keeps ignoring `ToolCallDelta`.

These are pure functions with unit tests. Reject a tool-use block in a user message, and a tool
result in an assistant message, with a clear error.

```gitcommit
feat(provider): convert tool schemas, calls, and results for rig

the one place rig's tool types meet ours. keeping it in pure
conversion functions means rig's api churn is absorbed and tested here
without any network.
```

#### 1.8 Offer tools through the rig adapter

`build_completion_request` forwards `tools` and maps `ToolChoice` onto rig's tool choice.
`complete` and `stream` report `StopReason::ToolUse` whenever the response contains a tool call.
While in this function, forward `top_p` too, since it is silently dropped today.

```gitcommit
feat(provider): offer tools and report tool use through the rig adapter

the adapter has always sent an empty tool list. rig exposes no
normalized stop reason, so tool use is inferred from the content, the
same way the adapter already infers end of turn. top_p was also being
dropped on the floor and is now forwarded.
```

#### 1.9 Replay tool calls in the default stream

`Provider::stream`'s default implementation turns `ContentBlock::ToolUse` into
`StreamEvent::ToolUse`, so any provider that only implements `complete`, including the mock, streams
tool calls faithfully.

```gitcommit
feat(provider): replay tool calls in the default stream

the default stream rebuilt events only from text and thinking blocks,
so a completion-only provider would silently lose its tool calls when
streamed.
```

#### 1.10 Script tool calls in the mock provider

- `MockProvider::respond_tool_use(name, arguments)` and `respond_tool_uses(Vec<(name, arguments)>)`
  give each call a deterministic id (`call-1`, `call-2`, and so on) and set `StopReason::ToolUse`.
- `respond_text_and_tool_use` covers the "let me check" case.

```gitcommit
feat(provider): script tool calls in the mock provider

every harness test drives the loop through scripted tool calls, so the
mock needs to produce them before the loop exists.
```

#### 1.11 Retry transient provider failures

- Add `backon` and `tracing`.
- `provider/retry.rs`: `RetryPolicy { max_attempts, base_delay, max_delay }` and a
  `RetryingProvider<P>` decorator. It retries only when `AiError::is_transient()` holds. For
  `stream`, it retries only the initial call, never after the first event has been yielded.
- Tests use millisecond delays.

```gitcommit
feat(provider): retry transient provider failures

a 5xx or rate limit from a provider currently fails the reply
outright. retrying a stream is limited to opening it, because
replaying text the user has already seen would duplicate it on
screen.
```

#### 1.12 Add a model pricing table

- `provider/pricing.toml`, embedded with `include_str!`, covering the models `docs/chat.md` mentions.
- `Pricing { input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok }`, and
  `estimate_cost(&ModelRef, &Usage) -> Cost`. Reasoning tokens bill at the output rate.
- An unknown model costs `Cost(0)` and logs a `warn!` once per model.

```gitcommit
feat(provider): add a model pricing table

turn budgets need a cost per turn, and later milestones record and cap
spend. prices live in an embedded toml file so they can be updated
without touching code. an unknown model warns rather than failing,
since a missing price should never take chat down.
```

### Tools

#### 1.13 Add risk tiers

`munibot_ai/src/tools.rs`, with the `tools/` directory and `tools/tier.rs`:

- `RiskTier` is `Safe`, `NetworkRead`, `BotData`, `Sandbox`, or `Privileged`, ordered by risk.
- Doc comments explain each tier. `BotData` is reserved: no tool uses it yet.

```gitcommit
feat(tools): add risk tiers

a tool's tier and the tier its invoker was granted decide together
whether the model is even shown it. ordering the tiers lets one
comparison answer that.
```

#### 1.14 Add the tool context

- Add `tokio-util`.
- `tools/context.rs`: `UserId(i64)`, which wraps `users.id`.
- `ToolCtx { user_id: Option<UserId>, granted: RiskTier, cancel: CancellationToken }`.
- `ToolCtx::require_tier(RiskTier) -> Result<(), AiError>`.

```gitcommit
feat(tools): add the tool context

carries who a tool is acting for and what they are allowed to do.
authority comes from the invoking human through this context, never
from anything the model asks for, and every risky tool re-checks it
at the point of use.
```

#### 1.15 Add the tool trait and outcome

- `tools/tool.rs`: `#[async_trait] trait Tool: Send + Sync`, with:
  - `name`
  - `description`
  - `tier`
  - `schema() -> ToolSchema`
  - `is_serial()`, defaulting to `false`
  - `async invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome`
- `ToolOutcome` is one of:
  - `Ok(String)`
  - `Err(String)`: model-visible, so the model can recover.
  - `Fatal(AiError)`: ends the turn.

```gitcommit
feat(tools): add the tool trait and outcome

a recoverable error goes back to the model so it can try something
else, while a fatal one ends the turn. most tool failures are the
first kind, and conflating them would make the model give up far too
easily.
```

#### 1.16 Add the tool registry

- `tools/registry.rs`: `ToolRegistry`, holding `Arc<dyn Tool>` by name, with `register`, `get`, and
  `schemas_for(granted: RiskTier) -> Vec<ToolSchema>`.
- Registering a duplicate name panics, since it's a programming error.
- Add table tests for tier filtering.

```gitcommit
feat(tools): add the tool registry

the loop looks tools up by name, and a model is only ever shown tools
its invoker's tier permits. filtering at the schema level means a
forbidden tool is not just refused but invisible.
```

### Harness

#### 1.17 Add turn budgets

- `munibot_ai/src/harness.rs` with `harness/`.
- `harness/budget.rs`: `Budget { max_iterations, max_wall_clock, max_cost }`, all optional.
  `Default` is 8 iterations, 60 seconds, and $0.25.
- `BudgetTracker` records usage and cost per iteration. `check()` returns `Option<BudgetLimit>`,
  naming which limit tripped.

```gitcommit
feat(harness): add turn budgets

a tool loop can call a model many times per message, and with no
ceiling one message can cost dollars. the conservative defaults make
an unconfigured deployment cheap rather than surprising.
```

#### 1.18 Add harness events

`harness/event.rs`:

- `HarnessEvent` variants:
  - `TextDelta(String)`
  - `ToolStarted { id, name }`
  - `ToolFinished { id, name, ok, duration }`
  - `Truncated(BudgetLimit)`
  - `Finished { usage, cost, iterations }`
- Failures arrive as `Err` items on the stream, not as an event.
- Derive `Clone` and `Debug`.

```gitcommit
feat(harness): add harness events

adapters render a turn's progress from these events, without knowing
how the loop works. tool start and finish are part of the vocabulary
from the beginning so the loop never ships without them.
```

#### 1.19 Add turn request and outcome types

- `TurnRequest { model, system, history: History, params, budget, ctx: ToolCtx }`.
- `TurnOutcome { text, transcript: Vec<Message>, usage, cost, iterations, truncated: Option<BudgetLimit> }`.
  `transcript` holds every message the turn appended, so a caller that persists history can store
  tool calls too.

```gitcommit
feat(harness): add turn request and outcome types

a turn's outcome carries its full transcript, not just its final
text, so that saved conversations can later keep tool calls and their
results without changing the loop.
```

#### 1.20 Add the agent loop

- `harness/run.rs`: `Harness { provider: Arc<dyn Provider>, tools: Arc<ToolRegistry> }`.
- `run_turn_streamed(self: Arc<Self>, TurnRequest) -> BoxStream<'static, Result<HarnessEvent, AiError>>`
  works as follows:
  1. Stream a completion.
  2. Forward text deltas.
  3. Collect tool calls.
  4. Invoke each tool in order, emitting `ToolStarted` and `ToolFinished`.
  5. Append the assistant message and the tool-result message.
  6. Loop until a response has no tool calls.
- An unknown tool name becomes a `ToolOutcome::Err` that lists the available tools.
- `run_turn` folds the stream into a `TurnOutcome`.
- `MockProvider` tests cover:
  - plain text;
  - one tool call;
  - two sequential iterations;
  - an unknown tool;
  - a fatal tool error;
  - `ToolStarted` arriving before `ToolFinished`, between the surrounding text deltas.

```gitcommit
feat(harness): add the agent loop

the core of the harness: call the model, run any tools it asks for,
feed the results back, and repeat until it answers. streaming is the
one implementation and run_turn folds over it, so the two can never
disagree about what a turn does.
```

#### 1.21 Run independent tool calls concurrently

Calls from one response run with `join_all`, and their results keep call order. Tools whose
`is_serial()` is true run one at a time after the concurrent batch. `ToolStarted` fires for every
call as soon as the batch starts, not when each one returns.

```gitcommit
feat(harness): run independent tool calls concurrently

a model that asks for three lookups at once should not wait for them
one after another. results keep call order because providers match
them up positionally as well as by id.
```

#### 1.22 Validate tool arguments before invoking

- Add `jsonschema`.
- Check arguments against the tool's schema before calling `invoke`. A failure becomes an
  `is_error` tool result naming the problem, and counts against `max_tool_retries` (default 3) for
  that turn.

```gitcommit
feat(harness): validate tool arguments before invoking

models regularly produce slightly wrong arguments. handing back the
validation error lets the model fix its own call, which is far more
robust than every tool defending against malformed input on its own.
```

#### 1.23 Enforce turn budgets

- Check the tracker before every provider call and after every response.
- When a limit trips with time and money still left, make one last call with `ToolChoice::None` so
  the model answers with what it has, then emit `Truncated`. When time or money is what ran out,
  emit `Truncated` straight away.
- Test each limit.

```gitcommit
feat(harness): enforce turn budgets

a turn that runs out of iterations mid-research would otherwise end
with no answer at all. one final tool-free call turns that into a
partial answer, which is better than none, unless it is time or
money that ran out.
```

#### 1.24 Cancel turns cleanly

- The provider stream and every tool invocation race `ctx.cancel` with `tokio::select!`.
- Dropping the returned stream cancels the token through a `DropGuard`.
- Cancellation yields `Err(AiError::Cancelled)` promptly.
- Test with a tool that sleeps forever.

```gitcommit
feat(harness): cancel turns cleanly

a reader who closes the page, or presses stop, should end the turn
and any tool it is waiting on immediately rather than letting it run
on and spend money nobody will see the result of.
```

### Built-in tools and configuration

#### 1.25 Add the current time tool

- Add `chrono` and `chrono-tz`.
- `tools/builtin/current_time.rs`: takes an optional IANA `timezone` and returns an RFC 3339
  timestamp plus a human-readable one. Tier `Safe`.
- `ToolRegistry::builtin()` registers it.

```gitcommit
feat(tools): add the current time tool

models have no clock. it is also the smallest possible real tool,
which makes it the right one to prove the whole path end to end.
```

#### 1.26 Add budget settings to the ai config

- `AiConfig.budget: BudgetConfig { max_iterations, max_wall_clock, max_cost_usd }`. Each field is
  optional and falls back to `Budget::default()`. Durations are parsed by `humantime-serde`.
- `BudgetConfig::resolve() -> Budget`.
- Test that a missing `[ai.budget]` table yields the defaults.

```toml
[ai.budget]
max_iterations = 8
max_wall_clock = "60s"
max_cost_usd = 0.25
```

```gitcommit
feat(config): add turn budget settings

operators need to tune how much one message may cost without
recompiling. every field is optional so existing config files keep
working unchanged.
```

### The chat on the harness

#### 1.27 Run chat replies through the agent loop

- `Chat` holds a `Harness` built from a `RetryingProvider` around the resolved provider, plus
  `ToolRegistry::builtin()`, and the resolved `Budget`.
- `Chat::reply(history, ctx: ToolCtx)` now returns
  `BoxStream<'static, Result<HarnessEvent, AiError>>`.
- In the same commit:
  - `munibot/src/ai.rs::build` passes the budget config through.
  - `chat_stream` builds a `ToolCtx` for the signed-in user (`granted: RiskTier::Safe`, plus a fresh
    token).
  - `ChatEvent::stream_from_reply` maps `TextDelta` and ends on `Finished`, ignoring every other
    event for now.

```gitcommit
feat: run chat replies through the agent loop

chat stops being a single provider call and becomes a harness turn,
with retries, a budget, and the built-in tools. the reply's signature
changes across the crates that call it, so they move together; tool
activity reaches the page in the next commit.
```

#### 1.28 Tell munibot what his tools are for

`prompts/conversation.md` stops claiming he has no tools. It says that he can check the time and
should use tools when they help, and still says plainly that he can't browse, remember past chats,
or run code. Add a `conversation.rs` test asserting the prompt doesn't say "no tools".

```gitcommit
feat(ai): tell munibot what his tools are for

the prompt still said he had no tools at all, which would make him
refuse to check the time while holding a tool that does exactly that.
```

#### 1.29 Add tool activity to chat events

`munibot_api/src/chat/event.rs`:

- `ChatEvent::ToolStarted { id, name }`.
- `ChatEvent::ToolFinished { id, ok, duration_ms }`.
- `ChatEvent::Truncated(String)`, a friendly explanation.
- Test the `HarnessEvent` to `ChatEvent` mapping directly, since it is a pure function.

```gitcommit
feat(event): add tool activity to chat events

the page needs to see tools start and finish while a turn runs, and to
learn when a reply was cut short by a budget rather than finished
naturally.
```

#### 1.30 Show tool activity while munibot works

- `munibot_gui/src/components/chat/tool_activity.rs`: a compact strip above the live reply. Each
  tool shows its name, a spinner while running, then a check or cross with its duration.
- When the turn finishes, the strip stays attached to that reply in page state. It is never sent
  back to the server.
- A truncated reply shows a small note saying why.

```gitcommit
feat(gui): show tool activity while munibot works

a turn that spends ten seconds on tools looks broken unless you can
see what it is doing. the strip makes that work visible and keeps it
next to the answer it produced.
```

#### 1.31 Add a stop button

- While a reply is running, the composer's send button becomes a stop button. Pressing it drops the
  event stream, which closes the SSE connection, so the server drops the harness stream and cancels
  the turn.
- Partial text is kept as the reply, with a "stopped" note.

```gitcommit
feat(gui): add a stop button

a wrong or runaway reply should be stoppable from the page. dropping
the stream is enough, because the server cancels the turn the moment
its stream is dropped.
```

#### 1.32 Document tools and budgets

- `docs/chat.md`: tools exist, which tools there are, the `[ai.budget]` table, and the stop button.
  Keep "no saved conversations, no rate limits".
- `docs/notes/rig-provider-findings.md`: add that tool calls stream whole and that tool use is
  inferred.

```gitcommit
docs: document chat tools and turn budgets

the chat guide still said munibot has no tools, and the rig notes
didn't record how tool calls come through the adapter.
```

---

## Definition of done

- Asking "what time is it in Tokyo?" in `/chat` shows `current_time` running and then finishing in
  the activity strip, followed by a correct answer.
- With `max_iterations = 1`, a turn that wants two tool calls still produces an answer, plus a note
  that it was cut short.
- Pressing stop during a turn ends it within a second, including when a tool is mid-call.
- A scripted 503 followed by success makes the turn succeed, with no duplicated text.
- Every harness behaviour is tested against `MockProvider`. `devenv test` passes with no network.

## Deliberately deferred

- **Web tools.** `web_search` and `web_fetch` are milestone 02, along with the untrusted-content
  wrapper they need.
- **Saving tool transcripts.** The full transcript exists in `TurnOutcome`, but has nowhere to go
  until milestone 03.
- **Extended thinking with tools.** `thinking_budget` isn't forwarded, and `ContentBlock::Thinking`
  carries no signature. Anthropic requires signed thinking blocks to be echoed back during tool use,
  so turning thinking on means adding the signature first.
- **Per-user limits.** A budget bounds one turn, but nothing bounds how many turns a person runs.
  That is milestone 04, and it's why `docs/chat.md` keeps warning that anyone signed in can spend the
  provider key.
- **Handoff schemas.** Structured terminal output is only needed by the autonomous pipeline, so it
  arrives with it in milestone 10.
