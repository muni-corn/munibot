# AI harness pre-flight findings

Results of the pre-flight checks listed in `docs/plans/ai/milestone-1-conversation.md`, run before any
implementation work started.

**Verified on:** `rustc 1.99.0-nightly (da86f4d07 2026-07-24)`, edition 2024, resolver 3.

**Headline:** the stack works, but **two assumptions in the original plan were wrong**. `rig-core` 0.41
has no `DynClientBuilder`, and its `CompletionModel` trait is not object-safe. Both are worked around
by our own `Provider` trait, which is exactly the boundary that decision was made to protect. The plan
documents have been corrected.

## Summary

| Check                                      | Result                                                      |
| ------------------------------------------ | ----------------------------------------------------------- |
| `rig-core` 0.41 builds on pinned nightly   | Pass                                                        |
| `DynClientBuilder` exists                  | **Fail — does not exist in 0.41**                           |
| `CompletionModel` is object-safe           | **Fail — has associated types, `Clone`, and RPITIT**        |
| Object-safe wrapper is possible            | Pass — verified by compilation                              |
| Runtime `provider:model` string resolution | Pass — via a hand-written match                             |
| Per-provider feature flags                 | **None exist — all providers are always compiled in**       |
| Low-level tool calling available           | Pass                                                        |
| Low-level streaming available              | Pass                                                        |
| New dependency conflicts                   | None — the lockfile already carries every version rig wants |
| `bollard` 0.21 builds on pinned nightly    | Pass                                                        |
| podman available for sandbox work          | **Not installed — deferred to milestone 3**                 |
| Exa API shape matches plan assumptions     | Pass, and better than assumed                               |

## 1. `DynClientBuilder` does not exist in rig-core 0.41

The plan's provider-abstraction rationale cited `DynClientBuilder` for resolving a provider from a
string at runtime. It is not in the 0.41 source. The API documented at `docs.rig.rs` describes an
older release, and `docs.rs/rig-core/latest/rig_core/client/builder/struct.DynClientBuilder.html`
returns 404.

What 0.41 actually exposes in `client/mod.rs` is a generic `Client<Ext, H>` with `Provider`,
`Capability`, `Capabilities`, and `ProviderBuilder` traits — a compile-time composition design, not a
runtime one.

## 2. `CompletionModel` is not object-safe

```rust
pub trait CompletionModel: Clone + WasmCompatSend + WasmCompatSync {
    type Response: ...;
    type StreamingResponse: ...;
    type Client;
    fn make(client: &Self::Client, model: impl Into<String>) -> Self;
    fn completion(&self, request: CompletionRequest)
        -> impl Future<Output = Result<CompletionResponse<Self::Response>, CompletionError>>;
    fn stream(&self, request: CompletionRequest)
        -> impl Future<Output = Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>>;
}
```

Three separate blockers: three associated types, a `Clone` supertrait, and `impl Future` in return
position. `Box<dyn CompletionModel>` is impossible.

### The verified workaround

A generic adapter over any concrete rig model, implementing our own object-safe trait with boxed
futures. This compiles, and `Arc<dyn Provider>` holding two different providers works:

```rust
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn complete<'a>(&'a self, req: Turn) -> BoxFuture<'a, Result<Vec<Block>, String>>;
    fn stream<'a>(&'a self, req: Turn) -> BoxFuture<'a, Result<BoxStream<'a, StreamEvent>, String>>;
}

struct RigProvider<M: CompletionModel> { name: String, model: M }

impl<M> Provider for RigProvider<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send,
    M::StreamingResponse: Send,
{ /* ... */ }
```

Runtime resolution becomes a match with one arm per supported provider:

```rust
fn resolve(model_ref: &str) -> Result<Arc<dyn Provider>, String> {
    let (provider, model) = model_ref.split_once(':').ok_or(/* ... */)?;
    match provider {
        "anthropic" => {
            let client = anthropic::Client::from_env()?;
            Ok(Arc::new(RigProvider { name: provider.into(), model: client.completion_model(model) }))
        }
        "openai" => { /* same shape */ }
        other => Err(format!("unknown provider {other:?}")),
    }
}
```

### Consequences

- Adding a provider is a one-arm code change, not configuration. Acceptable, but it means the set of
  supported providers is fixed at compile time even though the _choice_ among them is configuration.
- `anthropic::Client::from_env()` already produces the error the plan wanted:
  `environment variable "ANTHROPIC_API_KEY" is not set or is invalid`.
- Each arm is a few lines, so covering Anthropic, OpenAI, OpenRouter, Ollama, and Gemini is cheap.

## 3. No per-provider feature flags

`rig-core` 0.41's features are only about the HTTP stack and optional parsers:

```
audio, default (= reqwest + derive + rustls), derive, epub, image, native-tls,
pdf, rayon, reqwest, reqwest-middleware*, rustls, socks, test-utils, websocket*
```

All 26 provider modules compile unconditionally. The plan's proposed `anthropic` / `openai` /
`openrouter` / `ollama` feature flags on `munibot_ai_provider` are therefore impossible to forward to
rig, and have been removed. We can still gate our _own_ match arms behind our own features later if
compile time becomes a problem, but it will not reduce rig's own build.

## 4. The low-level API has everything the harness needs

Confirmed present and compiling:

| Need                | rig 0.41                                                                                |
| ------------------- | --------------------------------------------------------------------------------------- |
| Tool declaration    | `ToolDefinition { name, description, parameters: Value }`                               |
| Tool choice         | `ToolChoice { Auto, None, Required, Specific { function_names } }`                      |
| Response content    | `AssistantContent { Text, ToolCall, Reasoning, Image }`                                 |
| Tool call           | `ToolCall { id, call_id: Option<String>, function: ToolFunction, .. }`                  |
| Response envelope   | `CompletionResponse { choice: OneOrMany<_>, usage, raw_response, message_id }`          |
| Streaming           | `StreamingCompletionResponse<R>: Stream<Item = Result<StreamedAssistantContent<R>, _>>` |
| Streamed tool calls | `StreamedAssistantContent { Text, ToolCall { .. }, ToolCallDelta { .. }, .. }`          |

Two notes:

- `ToolCall` has both `id` and an optional `call_id`. Some providers correlate tool results by the
  latter. The conversion layer must prefer `call_id` and fall back to `id`, or tool results will fail
  to correlate on those providers.
- `Reasoning` gives us thinking blocks for free, which maps onto the planned
  `ContentBlock::Thinking`.

## 5. Structured output can suppress tool calls — handoff must stay a tool

`CompletionModel` carries this method:

```rust
/// Whether this provider's native structured output (`output_schema` ->
/// `format`/`response_format`) composes with tool calls in the same
/// multi-turn request without suppressing them.
fn composes_native_output_with_tools(&self) -> bool { false }
```

rig's own comment references upstream issue #1928: native structured output can make a model emit
schema JSON _instead of_ calling its tools, and the safe default is to assume it does.

This independently validates the plan's design decision to implement the pipeline handoff as an
injected `handoff` **tool** rather than as native structured output. Do not be tempted to switch to
`output_schema` for handoffs — it would silently break every agent that also needs real tools, which
is all of them.

## 6. No new dependency conflicts

`rig-core` 0.41 resolves to `reqwest` 0.13.4, `http` 1.4.2, `hyper` 1.11.0, `tokio` 1.53.1, and
`schemars` 1.2.2.

munibot's `Cargo.lock` **already** contains both `reqwest` 0.12.28 and 0.13.4, plus matching `http`,
`hyper`, and `tokio` versions. So rig introduces no new duplicate and no version bump. `schemars` 1.2
matches what the plan already specified.

The probe crate resolved to 237 packages total, most of which munibot already builds.

## 7. bollard builds, but podman is not installed

`bollard` 0.21 compiles on the pinned nightly, and `Docker::connect_with_socket_defaults()`
constructs successfully.

However `podman` is **not** on this machine and `$XDG_RUNTIME_DIR/podman/` does not exist. So:

- Milestone 3 commit 99 (adding podman to `devenv.nix`) is genuinely required, not a formality.
- Actual container lifecycle behaviour is **unverified**. Re-run this check after commit 99 and before
  writing commit 111.

## 8. Exa's API is better than the plan assumed

Confirmed against the OpenAPI spec at `docs.exa.ai`:

- `POST https://api.exa.ai/search` and `POST https://api.exa.ai/contents`, authenticated with an
  `x-api-key` header.
- **`/search` accepts an inline `contents` object**, so search and extraction are a single round trip:

  ```json
  { "query": "...", "contents": { "highlights": true, "text": true } }
  ```

- Responses include a **`costDollars`** breakdown, so search spend can be accounted exactly rather
  than estimated.
- `/search` also supports `text/event-stream`.

Two plan improvements follow:

1. `web_search` should request inline `contents` so a research loop does not need a second `web_fetch`
   call per result. `web_fetch` remains, for URLs the user supplies directly.
2. `costDollars` should feed the `ai_usage` cost column alongside model cost, so the usage dashboard
   reflects true spend rather than model spend only.

**Not verified live:** no `EXA_API_KEY` was available, and Exa is not yet declared in
`secretspec.toml`. The request and response shapes above come from the published OpenAPI spec, not
from a live call. Re-verify when the key lands in commit 3.

## 9. Incidental finding: rig has OpenTelemetry support

`CompletionRequest` carries a `record_telemetry_content: bool` field, and rig advertises full GenAI
semantic convention compatibility. Worth investigating for milestone 5 commit 162 instead of hand
-rolling every span attribute.

## Reproducing

The probe was a throwaway crate outside the repository. To rebuild it, create a binary crate on
edition 2024 with `rig-core`, `tokio`, `futures`, and `serde_json`, and paste the adapter from section 2. It requires no API keys — resolution failure on a missing key is itself a passing result.
