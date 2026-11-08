# rig findings

What `rig` 0.46 gives munibot, what it doesn't, and the sharp edges found while building on it.
Read this before adding anything to `munibot_ai`: the rule is to use rig's type whenever one
exists, and to write our own only for something rig genuinely lacks.

## History

munibot first built on `rig-core` 0.41. That version's `CompletionModel` wasn't object-safe, it had
no runtime model selection, no normalized stop reason, and no response headers in errors. So
`munibot_ai` grew its own `Provider` trait, a generic `RigProvider<M>` adapter, a `provider:model`
`ModelRef`, a resolver, a mock, and a full set of mirrored message, request, stream, usage, and
error types: about 2,300 lines that existed only to work around rig.

rig 0.43 through 0.46 closed every one of those gaps, so all of it was deleted. `munibot_ai` now
re-exports rig as `munibot_ai::rig`, and every crate uses rig's types through that one path, so
there's never a second rig version in play.

## What munibot uses from rig-core

| Need                        | rig 0.46                                                                                                                |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| A model chosen at runtime   | `DynModel<operation::Completion>`: cheap to clone, `Send + Sync`, `call` and `stream`                                   |
| Selecting a model by string | `catalog::Catalog::builtin().connect("vendor/model")`, which reads the vendor's key from the environment                |
| Messages                    | `completion::message::Message` (`System`, `User`, `Assistant`), with `UserContent` and `AssistantContent`               |
| Requests                    | `completion::CompletionRequest`, built from a `Vec<Message>`, with `.preamble`, `.max_tokens`, `.tools`, `.tool_choice` |
| Streaming                   | `streaming::{Item, StreamEvent}`; `Streamed::finish()` folds the reply into a `CompletionResponse`                      |
| Finish reasons              | `completion::FinishReason` and `message::StopReason`                                                                    |
| Usage and cost              | `completion::{Usage, Cost}`, with cost filled in from the catalog's prices when the provider doesn't report it          |
| Model facts                 | `catalog::ModelSpec`: pricing, context and output limits, input modalities (vision), reasoning support                  |
| Errors                      | `ProviderError`, with `is_retryable()`, `provider_response_status()`, and `report()`                                    |
| Retry hints                 | `ErrorReport::retry_after(now)`, read from `retry-after-ms`, `retry-after`, or the body                                 |
| Context overflow            | `ErrorReport::is_context_overflow()`                                                                                    |
| Tests                       | `test_utils::MockCompletionModel` (feature `test-utils`), scripting text, tool calls, streams, and errors               |

## Sharp edges

**References are `vendor/model`, never `vendor:model`.** `Catalog::connect("anthropic:claude-opus-5")`
fails with `ConnectError::Malformed`. Models the catalog doesn't list still connect, as long as rig
knows the vendor.

**The system prompt counts as history.** `CompletionRequest::from(vec![]).preamble(..)` holds one
message, so rig will send it. `Chat::reply` refuses an empty history itself, using rig's own
`ProviderError::request`.

**Ollama through the catalog has no base-url variable.** It always targets `localhost:11434`. A
remote daemon needs `ConnectOptions::base_url`, which `Chat::connect` doesn't expose yet.

**The stream stall timeout is fixed.** A streamed reply that sends nothing for five minutes fails
with the retryable `ProviderError::Stalled`. `Model::with_idle_timeout` can change that, but only
before the model is erased, and `Catalog::connect` returns an erased one.

**Options can turn into refusals.** Once any `GenerationOptions` field is set, a model that can't
honour another field (Claude Opus 5 refuses `temperature`, per its catalog row) fails with
`ProviderError::UnsupportedOption`, unless `OnUnsupported::Ignore` is set. `DynModel::check` reports
every refusal without sending anything.

**`Usage.input_tokens` already includes cache reads and writes.** Never add
`cached_input_tokens` on top of it. Streamed usage isn't a stream item: read it from
`Streamed::finish()`.

**`ProviderError` is large** (224 bytes), which trips clippy's `result_large_err`. rig's own
signatures return it unboxed, so munibot allows the lint where it passes the error through, rather
than boxing it.

**`ProviderError`'s text is the provider's raw reply.** Never show it to a user. The chat page shows
a sentence chosen from rig's classification instead (`munibot_api/src/chat/failure.rs`).

**rig-core brings reqwest 0.13** (through `rig-reqwest`, behind its `reqwest` feature), alongside
the reqwest 0.12 the rest of the workspace uses. Two copies compile until the workspace moves to
0.13.

## What rig 0.46 doesn't have

These are the gaps munibot still fills itself. The plans in `docs/plans/ai/` say where each one
lands.

- **Automatic retries.** rig classifies errors and reads `Retry-After`, but never retries. The
  ready-made `RetryPolicy` lives in `rig-ecs`, which pins Bevy, so it isn't worth depending on.
- **rig-agent's loop lacks budgets on cost or time.** `max_turns` (default 1) is the only limit, and
  exhausting it is an error rather than a final answer. rig's own `run_budget` example shows how to
  add limits as an `AgentHook`.
- **No live tool start and finish events.** `MultiTurnStreamItem::ToolResult` arrives only after the
  whole tool batch settles. Live progress comes from the `on_dispatch` and `on_outcome` hooks.
- **No serial tools.** `AgentRunner::tool_concurrency(n)` treats every call alike. Footprint-aware
  scheduling exists only in `rig-ecs`.
- **No schema validation before dispatch in rig-agent.** `tool::validate` and
  `tool::feedback::arguments_refusal` exist in rig-core, so an `on_dispatch` hook can apply them.
- **Unknown tools fail the run by default.** An `on_invalid_tool_call` hook returning
  `InvalidToolCallAction::retry(feedback::unknown_tool_feedback(..))`, plus
  `max_invalid_tool_call_retries`, makes them model-visible.
- **No risk tiers or authorization.** `ToolAnnotations` holds a fixed set of MCP hints.
- **`Agent::into_tool` delegation shares nothing with the parent run**: no budget, no usage, and
  no depth limit.
- **Structured output validates only top-level required fields** in rig-agent's output-tool mode.
- **No web search, web fetch, or clock tools.** The only built-in tool is `think`.
- **`rig-memory` targets a single local agent.** It's an append-only JSONL log store with no
  owners, no listing, and whole-history loads, so it doesn't fit a multi-user server backed by
  MySQL. Its ideas are still worth copying: `conversation::pairing::safe_start`, compaction's cut
  rule, and its summary prompt structure.
