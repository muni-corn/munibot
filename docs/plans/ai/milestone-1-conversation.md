# Milestone 1 — conversation

**Outcome:** you mention munibot in Discord and he answers, in character, streaming his reply, using
tools when he needs them, staying inside a cost budget.

This milestone builds the entire foundation: types, provider abstraction, tool system, agent loop,
context management, persona configuration, and one platform adapter. Nothing here is throwaway —
milestones 2 through 5 add to it rather than replace it.

**Phases 0 through 8, commits 1 through 65.**

## Pre-flight checks

Not commits, but do these before phase 1 or you may build on sand.

1. Verify `rig-core` 0.41 and `bollard` compile on the pinned nightly toolchain. `munibot_discord`
   requires nightly for `#![feature(never_type)]`, so the whole workspace is nightly-only.
2. Confirm which `rig-core` feature flags are needed for Anthropic and OpenAI, and whether
   `DynClientBuilder` is behind one.
3. Get an Exa API key and confirm the `/search` and `/contents` endpoints return what the plan
   assumes.

---

## Phase 0 — preparation

| #   | Commit                                                 | Description                                                                                                                                                                                                                                                        |
| --- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | `docs(ai): add agent harness implementation plan`      | Add `docs/plans/ai/` with `overview.md` and the five milestone files.                                                                                                                                                                                              |
| 2   | `build(deps): add ai provider workspace dependencies`  | Add to `[workspace.dependencies]`: `rig-core` 0.41, `schemars` 1.2, `futures` 0.3, `tokio-util` (for `CancellationToken`), `backon` or equivalent for retry, and `jsonschema` for handoff validation. Do not add them to any crate yet.                            |
| 3   | `build(secretspec): add ai provider api key variables` | Add `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, and `EXA_API_KEY` to `secretspec.toml`, all with `required = false` so development without keys still boots. Mirror them into `.env.example` with empty values and a comment naming the provider. |

---

## Phase 1 — `munibot_ai_types`

Provider-neutral domain types. No I/O, no async, no HTTP. Everything else in the system speaks these
types, which is what makes the provider swappable.

Every type derives `Serialize`, `Deserialize`, `Clone`, `Debug`, and `PartialEq`. Types that cross
into a tool schema also derive `JsonSchema`. Every commit adds colocated roundtrip tests.

| #   | Commit                                                      | Description                                                                                                                                                                                                                                                                                             |
| --- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 4   | `build(ai_types): create ai types crate skeleton`           | Create `munibot_ai_types/` with `Cargo.toml` depending only on `serde`, `serde_json`, `schemars`, `thiserror`, and `chrono`. Empty `lib.rs`. Add to workspace members.                                                                                                                                  |
| 5   | `feat(ai_types): add role and content block types`          | `Role` (`System`, `User`, `Assistant`, `Tool`) and `ContentBlock` (`Text`, `Image`, `ToolUse`, `ToolResult`, `Thinking`). `ToolUse` carries `call_id`, `name`, `arguments: Value`. `ToolResult` carries `call_id`, `content`, `is_error: bool`.                                                         |
| 6   | `feat(ai_types): add message and history types`             | `Message { role, content: Vec<ContentBlock> }` with constructors `Message::user`, `Message::assistant`, `Message::tool_result`. `History(Vec<Message>)` with `push`, `iter`, and a `token_estimate` method taking a `TokenCounter` closure.                                                             |
| 7   | `feat(ai_types): add model reference and parameter types`   | `ModelRef { provider: String, model: String }` with `FromStr` parsing `"anthropic:claude-opus-5"` and a `Display` that round-trips. Reject empty halves with a clear error. `ModelParams { temperature, top_p, max_tokens, thinking_budget }`, all `Option`.                                            |
| 8   | `feat(ai_types): add tool schema and definition types`      | `ToolSchema { name, description, input_schema: Value }` and a `from_schemars::<T>()` constructor so tools can derive their schema from a Rust struct.                                                                                                                                                   |
| 9   | `feat(ai_types): add completion request and response types` | `CompletionRequest { model, system, history, tools, params, tool_choice }` and `CompletionResponse { content: Vec<ContentBlock>, stop_reason, usage }`. `StopReason` covers `EndTurn`, `ToolUse`, `MaxTokens`, `StopSequence`, and `Refusal`. Add `CompletionResponse::tool_uses()` returning a slice.  |
| 10  | `feat(ai_types): add streaming event types`                 | `StreamEvent`: `TextDelta(String)`, `ThinkingDelta(String)`, `ToolUseStart { call_id, name }`, `ToolUseDelta(String)`, `ToolUseEnd`, `Usage(Usage)`, `Done(StopReason)`. Deliberately mirrors the union of Anthropic and OpenAI stream shapes so no provider needs a lossy mapping.                     |
| 11  | `feat(ai_types): add usage and cost accounting types`       | `Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens }` with `Add` and `Sum` impls so a multi-turn loop can total itself. `Cost(i64)` in micro-dollars — integer, never floating point, because these get summed and stored.                                                     |
| 12  | `feat(ai_types): add ai error type hierarchy`               | `AiError` with `thiserror`, using lowercase friendly messages with emoticons per `AGENTS.md`. Variants: `Provider`, `RateLimited { retry_after }`, `BudgetExceeded { limit }`, `Tool`, `SchemaViolation`, `Cancelled`, `Config`, `Other`. Add an `is_transient()` method that the retry layer consumes. |

---

## Phase 2 — `munibot_ai_provider`

The boundary that makes providers interchangeable. `rig-core` is a dependency of this crate and no
other.

| #   | Commit                                                         | Description                                                                                                                                                                                                                                                                                                                                                                                                    |
| --- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 13  | `build(ai_provider): create ai provider crate skeleton`        | Create `munibot_ai_provider/` depending on `munibot_ai_types`, `rig-core`, `tokio`, `futures`, `async-trait`, `tracing`, and the retry crate. Feature flags `anthropic`, `openai`, `openrouter`, `ollama` forwarding to rig's, all in `default`.                                                                                                                                                               |
| 14  | `feat(ai_provider): add provider trait definition`             | `#[async_trait] pub trait Provider: Send + Sync` with `fn name(&self) -> &str`, `async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, AiError>`, and `async fn stream(&self, req) -> Result<BoxStream<Result<StreamEvent, AiError>>, AiError>`. Default `stream` implementation wraps `complete` and emits one `TextDelta` plus `Done`, so a non-streaming provider is still usable. |
| 15  | `feat(ai_provider): add mock provider for testing`             | `MockProvider` holding a `VecDeque<CompletionResponse>` plus a recorded `Vec<CompletionRequest>`. Builder methods `respond_text`, `respond_tool_use`, `respond_error`. Panics with a clear message when the script runs dry. **Build this before the rig backend** — the whole test suite depends on it.                                                                                                       |
| 16  | `feat(ai_provider): add rig type conversion layer`             | `munibot_ai_provider/src/rig/convert.rs`: `Message` to rig's message type and back, `ToolSchema` to rig's `ToolDefinition`, rig's completion response to `CompletionResponse`, rig's streaming chunks to `StreamEvent`. Pure functions with unit tests. This file is where rig's churn is absorbed, so keep it isolated.                                                                                       |
| 17  | `feat(ai_provider): add rig backed provider implementation`    | `RigProvider` wrapping rig's `DynClientBuilder`. Resolve `ModelRef` to a boxed dynamic completion model, cache resolved clients in a `RwLock<HashMap>` keyed by provider name. Implement `complete` and `stream` through the conversion layer.                                                                                                                                                                 |
| 18  | `feat(ai_provider): add error classification for retries`      | Map rig and HTTP errors onto `AiError`, setting `is_transient()` correctly: 429 and 5xx and connection resets are transient, 4xx other than 429 are permanent. Extract `Retry-After` into `RateLimited { retry_after }`.                                                                                                                                                                                       |
| 19  | `feat(ai_provider): add retry policy with exponential backoff` | `RetryPolicy { max_attempts, base_delay, max_delay, jitter }` and a `RetryingProvider<P>` decorator that retries only when `is_transient()` holds and honours `retry_after`. Tests drive it with `MockProvider` scripted errors and a zeroed clock.                                                                                                                                                            |
| 20  | `feat(ai_provider): add model pricing table`                   | `Pricing { input_per_mtok, output_per_mtok, cache_read_per_mtok }` and `fn estimate_cost(model: &ModelRef, usage: &Usage) -> Cost`. Table loaded from a TOML file embedded with `include_str!` so prices update without code changes. Unknown models yield `Cost(0)` and log a warning rather than failing.                                                                                                    |
| 21  | `feat(ai_provider): add provider registry with key resolution` | `ProviderRegistry::from_env()` reading each `*_API_KEY`, registering only providers whose key is present, and logging which ones are available at startup. `resolve(&ModelRef)` returns a clear error naming the missing environment variable when a persona references an unconfigured provider.                                                                                                              |

---

## Phase 3 — `munibot_ai_tools`

The tool trait and registry. Built-in tools come in phase 5, once the harness can actually call them.

| #   | Commit                                               | Description                                                                                                                                                                                                                                                                                                                 |
| --- | ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 22  | `build(ai_tools): create ai tools crate skeleton`    | Create `munibot_ai_tools/` depending on `munibot_ai_types`, `async-trait`, `schemars`, `serde_json`, `tracing`, and `tokio`.                                                                                                                                                                                                |
| 23  | `feat(ai_tools): add risk tier and selection types`  | `RiskTier` (`Safe`, `NetworkRead`, `BotData`, `Sandbox`, `Privileged`) with an ordering. `ToolSelection` as an enum of `All`, `UpToTier(RiskTier)`, or `Named(Vec<String>)`, deserialized from the TOML list form where `"tier0"` through `"tier4"` expand to tiers and anything else is a tool name.                       |
| 24  | `feat(ai_tools): add tool context type`              | `ToolCtx` carrying the invoking user's internal `users.id`, platform identity, granted `RiskTier`, guild identifier, `CancellationToken`, and a `conversation_id`. Add `require_tier(RiskTier) -> Result<(), AiError>` so each tool guards itself in one line.                                                              |
| 25  | `feat(ai_tools): add tool trait and outcome types`   | `#[async_trait] pub trait Tool: Send + Sync` with `name`, `description`, `tier`, `input_schema`, and `async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome`. `ToolOutcome` is `Ok(String)`, `Err(String)` for model-visible recoverable failures, or `Fatal(AiError)` for failures that abort the turn.       |
| 26  | `feat(ai_tools): add tool registry with schema list` | `ToolRegistry` holding `HashMap<String, Arc<dyn Tool>>`. `register`, `get`, and `schemas_for(&ToolSelection, granted: RiskTier) -> Vec<ToolSchema>` which filters by both persona selection and granted tier, so a persona cannot be configured into a tier the invoker lacks. Table-driven tests for the filtering matrix. |

---

## Phase 4 — `munibot_ai_harness`

The agent loop. Types and budgets first, then behaviour, per the ordering rule in `AGENTS.md`.

| #   | Commit                                                     | Description                                                                                                                                                                                                                                                                                                                       |
| --- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 27  | `build(ai_harness): create ai harness crate skeleton`      | Create `munibot_ai_harness/` depending on `munibot_ai_types`, `munibot_ai_provider`, `munibot_ai_tools`, `tokio`, `tokio-util`, `futures`, `jsonschema`, and `tracing`.                                                                                                                                                           |
| 28  | `feat(ai_harness): add budget types`                       | `Budget { max_iterations, max_input_tokens, max_output_tokens, max_wall_clock, max_cost }`, all `Option`, with a `Default` that is deliberately conservative: 8 iterations, 60 seconds, 25 cents. `BudgetTracker` accumulating usage and returning `Result<(), AiError>` from `check()`.                                          |
| 29  | `feat(ai_harness): add harness event types`                | `HarnessEvent`: `TurnStarted { persona }`, `Thinking(String)`, `TextDelta(String)`, `ToolStarted { name }`, `ToolFinished { name, duration, ok }`, `IterationComplete { n, usage }`, `Handoff(Value)`, `TurnFinished { usage, cost }`, `Failed(AiError)`. Adapters render these; nothing else needs to know how the loop works.   |
| 30  | `feat(ai_harness): add turn request and outcome types`     | `TurnRequest { system, history, tools: ToolSelection, model, params, budget, handoff, ctx }` and `TurnOutcome { text, handoff, usage, cost, iterations }`.                                                                                                                                                                        |
| 31  | `feat(ai_harness): add agent loop with provider dispatch`  | `Harness { provider, tools }` and `async fn run_turn(&self, req) -> Result<TurnOutcome, AiError>`. Loop: build `CompletionRequest`, call the provider, and if the stop reason is not `ToolUse`, return the assembled text. Tests with `MockProvider` cover the single-turn text case.                                             |
| 32  | `feat(ai_harness): add tool call execution`                | On `StopReason::ToolUse`, look each call up in the registry, invoke it, and append a `ToolResult` block for every call. Unknown tool name yields a model-visible `ToolOutcome::Err` naming the available tools, not a hard failure — the model can correct itself.                                                                |
| 33  | `feat(ai_harness): add parallel tool call execution`       | Run independent tool calls concurrently with `futures::future::join_all`, preserving result order to match call order. Tools declare `fn is_serial(&self) -> bool` defaulting to `false`; serial tools run one at a time.                                                                                                         |
| 34  | `feat(ai_harness): add tool argument validation and retry` | Validate tool arguments against the tool's `input_schema` before invoking. On failure, return the validation error to the model as a `ToolResult` with `is_error: true` and count it against a separate `max_tool_retries`. This is the single highest-value robustness feature in the loop.                                      |
| 35  | `feat(ai_harness): add budget enforcement to the loop`     | Check `BudgetTracker` at the top of every iteration and after every provider response. On exhaustion, break cleanly and return whatever text exists with a truncation marker, rather than erroring — a partial answer beats no answer.                                                                                            |
| 36  | `feat(ai_harness): add cancellation support`               | Thread the `CancellationToken` from `ToolCtx` into the provider call and every tool invocation with `tokio::select!`. Cancellation returns `AiError::Cancelled` promptly and never leaks a running tool.                                                                                                                          |
| 37  | `feat(ai_harness): add handoff validation with retry`      | When `TurnRequest::handoff` is set, inject a `handoff` tool whose schema is the expected payload. A valid call terminates the turn with `TurnOutcome::handoff` populated. An invalid one returns the JSON schema validation error as a tool result and retries up to a budget. This is the mechanism the whole pipeline rests on. |
| 38  | `feat(ai_harness): add streaming turn execution`           | `run_turn_streamed(&self, req) -> BoxStream<HarnessEvent>` driving the same loop through `Provider::stream`, emitting events as they arrive. Implemented with `async_stream` or a channel plus a spawned task carrying a `tracing` span, per `docs/tracing.md`.                                                                   |

---

## Phase 5 — built-in tools

Tier 0 and tier 1 tools. Now that the loop exists, each one is independently testable.

| #   | Commit                                          | Description                                                                                                                                                                                                                                                                             |
| --- | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 39  | `feat(ai_tools): add current time tool`         | `current_time` taking an optional IANA timezone and returning a formatted timestamp. Tier `Safe`. Trivial, but it proves the whole tool path end to end and is genuinely useful, since models have no clock.                                                                            |
| 40  | `feat(ai_tools): add todo write tool`           | `todo_write` accepting a list of `{ content, status }` items, stored per conversation in memory for now. Tier `Safe`. Gives long research runs a scratchpad and makes progress legible to the user through `HarnessEvent`.                                                              |
| 41  | `feat(ai_tools): add untrusted content wrapper` | `wrap_untrusted(source: &str, body: &str) -> String` fencing external content in delimiters with an explicit warning that it is data and not instructions. Every tier 1 and above tool returns its payload through this. **Do this before the tools that need it.**                     |
| 42  | `feat(ai_tools): add exa api client`            | `munibot_ai_tools/src/exa.rs`: a struct-wrapped `reqwest::Client` with a `User-Agent` built from `env!("CARGO_PKG_VERSION")`, following the pattern in `munibot_discord/src/pluralkit/api.rs`. Methods `search` and `contents`, typed request and response structs, private error type. |
| 43  | `feat(ai_tools): add web search tool`           | `web_search` taking `query` and optional `num_results`, returning titles, URLs, and highlights through the untrusted wrapper. Tier `NetworkRead`. Fails soft: an Exa outage returns `ToolOutcome::Err` so the model can carry on without search.                                        |
| 44  | `feat(ai_tools): add web fetch tool`            | `web_fetch` taking a URL and optional `max_characters`, preferring Exa `contents` and falling back to `reqwest` plus readability extraction. Refuses non-HTTP schemes and private address ranges to block server-side request forgery. Tier `NetworkRead`.                              |

---

## Phase 6 — `munibot_ai_memory`

Traits and an in-memory implementation only. The diesel-backed store arrives in milestone 2, so
milestone 1 stays free of migrations.

| #   | Commit                                                    | Description                                                                                                                                                                                                                                                                              |
| --- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 45  | `build(ai_memory): create ai memory crate skeleton`       | Create `munibot_ai_memory/` depending on `munibot_ai_types`, `async-trait`, `chrono`, `tokio`, and `tracing`. No diesel dependency yet.                                                                                                                                                  |
| 46  | `feat(ai_memory): add conversation scope types`           | `ConversationId`, `Platform` (`Discord`, `Twitch`, `Web`), and `ConversationScope { platform, scope_key }` where `scope_key` is a channel, thread, or direct-message identifier. `Conversation { id, scope, persona_id, summary, last_active_at }`.                                      |
| 47  | `feat(ai_memory): add session store trait`                | `#[async_trait] pub trait SessionStore: Send + Sync` with `load_or_create(scope, persona) -> Conversation`, `append(conversation_id, Message)`, `history(conversation_id, limit) -> History`, `set_summary`, and `clear(conversation_id)`.                                               |
| 48  | `feat(ai_memory): add in memory session store`            | `InMemorySessionStore` over `RwLock<HashMap>` with a configurable per-conversation message cap. Used by tests everywhere and sufficient for the first Discord build.                                                                                                                     |
| 49  | `feat(ai_memory): add context assembly with token budget` | `assemble_context(store, scope, budget) -> History` walking messages newest-first until the token budget is spent, then reversing. Always keeps whole messages, never splits a tool-use and tool-result pair, and prepends the conversation summary when one exists. Table-driven tests. |

---

## Phase 7 — `munibot_ai` facade

The crate the rest of munibot actually imports. Configuration and personas, then the service handle.

| #   | Commit                                                  | Description                                                                                                                                                                                                                                                                                                    |
| --- | ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 50  | `build(ai): create ai facade crate skeleton`            | Create `munibot_ai/` depending on every `munibot_ai_*` crate plus `munibot_core`, `toml`, and `tracing`. `lib.rs` re-exports the types adapters need so a platform crate never imports six crates directly.                                                                                                    |
| 51  | `feat(ai): add persona types`                           | `PersonaId(String)`, `Persona` as specified in the overview, `MemoryPolicy` (`None`, `Conversation`, `User`), and `SandboxPolicy` (`Forbidden`, `Optional`, `Required`). Serde defaults on every optional field.                                                                                               |
| 52  | `feat(ai): add ai configuration section`                | `AiConfig { enabled, default_persona, prompt_dir, router, personas: HashMap<PersonaId, PersonaConfig> }` and an `ai: AiConfig` field on `munibot_core::Config`. `#[serde(default)]` on everything, a matching `Default` arm, and roundtrip tests following `munibot_core/src/config.rs:120`.                   |
| 53  | `feat(ai): add prompt template engine`                  | `PromptTemplate` with `{{variable}}` substitution, a `required_variables()` accessor, and a render that errors by naming every missing variable at once instead of one at a time. Unknown variables in the context are ignored; missing required ones are an error.                                            |
| 54  | `feat(ai): add companion persona prompt`                | `munibot_ai/prompts/companion.md`: munibot's voice for casual conversation and emotional support. States the instruction hierarchy, marks user content as untrusted, sets boundaries on advice, and defines the tone. **This file is the product.** Budget real time for it.                                   |
| 55  | `feat(ai): add writer and researcher persona prompts`   | `writer.md` for brainstorming and drafting, and `researcher.md` for multi-step research with mandatory citation of every claim traceable to a fetched source.                                                                                                                                                  |
| 56  | `feat(ai): add coder persona prompt`                    | `coder.md` for explaining code, reviewing pasted snippets, and debugging from a stack trace. No filesystem access in this milestone, so the prompt states plainly that it cannot run or modify anything yet.                                                                                                   |
| 57  | `feat(ai): add persona registry with prompt resolution` | `PersonaRegistry::load(&AiConfig)` resolving each persona's prompt from `prompt_dir` when present and falling back to the embedded default, validating that every referenced model has a configured provider, and returning an error listing every problem found. Fail at startup, never mid-conversation.     |
| 58  | `feat(ai): add output filter for platform safety`       | `filter_output(text, limits) -> String` stripping `@everyone`, `@here`, and role mentions, collapsing mass user mentions, enforcing a maximum length with a graceful ellipsis, and running `decancer` over the result. Applied by every adapter, tested independently.                                         |
| 59  | `feat(ai): add ai service handle`                       | `Ai { registry, harness, sessions, providers }` with `Ai::new(config, db)` and `async fn turn(&self, req: AiTurnRequest) -> Result<TurnOutcome, AiError>` plus a `turn_streamed` variant. `AiTurnRequest` names a persona explicitly; routing arrives in milestone 2. This is the only surface adapters touch. |

---

## Phase 8 — Discord adapter

The payoff. Thin translation between Discord and `Ai`, with no business logic of its own.

| #   | Commit                                                  | Description                                                                                                                                                                                                                                                                                                                                                                  |
| --- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 60  | `feat(discord): add ai chat event handler`              | `munibot_discord/src/handlers/ai.rs` implementing `DiscordEventHandler`. Triggers on a direct mention, a reply to munibot, or a direct message. Guards against reacting to itself the way `munibot_core/src/greeting.rs:32` does, and does a cheap in-memory trigger check before any database or provider call, per the rule in `docs/notes/gui-configuration-research.md`. |
| 61  | `feat(discord): add streaming response renderer`        | `munibot_discord/src/handlers/ai/render.rs`: send a placeholder, then edit it on a timer no faster than once per second to respect the five-edits-per-five-seconds channel limit, and edit once more at the end. Split anything over 2000 characters across follow-up messages at paragraph boundaries.                                                                      |
| 62  | `feat(discord): add tool activity indicator`            | Render `HarnessEvent::ToolStarted` as an italic status line above the partial response, replaced by the final text on completion. Turns a twenty-second research call from a silence into visible progress.                                                                                                                                                                  |
| 63  | `feat(discord): add ask command with persona selection` | `munibot_discord/src/commands/ai.rs` providing `/ask` with a `prompt` argument and an optional `persona` choice populated from the registry. Uses `ctx.defer()` for the slow path, mirroring `ventriloquize.rs:38`.                                                                                                                                                          |
| 64  | `feat(discord): add persona and reset commands`         | `/persona` to show or pin the channel's persona, and `/reset` to clear conversation context. Pinning is in-memory this milestone and becomes a database column in phase 9.                                                                                                                                                                                                   |
| 65  | `feat(munibot): register ai handler and commands`       | Add the AI handler to `DiscordMessageHandlerCollection` and the command provider to `DiscordCommandProviderCollection` in `munibot/src/bot.rs:29`. Construct `Ai` once in `main.rs` and share it as an `Arc`. Skip registration entirely when `ai.enabled` is false so a keyless deployment still boots.                                                                     |

---

## Definition of done

- Mentioning munibot in a Discord channel produces a streamed, in-character reply.
- `/ask persona:researcher` runs a multi-step tool loop with visible progress and cites its sources.
- A missing API key produces a clear startup error naming the variable, not a panic mid-conversation.
- Exceeding a budget truncates gracefully with a marker instead of erroring.
- `devenv test` passes with no network access.
- Setting `ai.enabled = false` removes the feature completely.

## Deliberately deferred

Conversation history does not survive a restart, there is no per-user memory, no auto-routing, no
Twitch support, and no settings interface. Those are milestone 2. Resisting them here is what keeps
this milestone shippable.
