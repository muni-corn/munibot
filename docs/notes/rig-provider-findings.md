# rig provider findings

What `rig-core` 0.41 does and doesn't offer, as found while building `munibot_ai`'s provider layer.
Each gap has a workaround in `munibot_ai/src/provider`.

**Headline:** `rig-core` 0.41 has no `DynClientBuilder`, and its `CompletionModel` trait is not
object-safe. Both are worked around by munibot's own `Provider` trait, which is the only boundary
the rest of the crate talks to.

## `DynClientBuilder` does not exist in rig-core 0.41

Older `rig` documentation describes `DynClientBuilder` for resolving a provider from a string at
runtime. It is not in the 0.41 source, and `docs.rs` returns 404 for it.

What 0.41 actually exposes in `client/mod.rs` is a generic `Client<Ext, H>` with `Provider`,
`Capability`, `Capabilities`, and `ProviderBuilder` traits: a compile-time composition design, not a
runtime one.

## `CompletionModel` is not object-safe

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

### The workaround

A generic adapter (`RigProvider<M>`) over any concrete rig model, implementing munibot's own
object-safe `Provider` trait. `Arc<dyn Provider>` can then hold any provider.

Runtime resolution of a `provider:model` string (`ProviderResolver`) becomes a match with one arm per
supported provider. Adding a provider is therefore a one-arm code change, not configuration: the set
of supported providers is fixed at compile time even though the _choice_ among them is configuration.

`anthropic::Client::from_env()` already produces a usable error for a missing key:
`environment variable "ANTHROPIC_API_KEY" is not set or is invalid`.

## No per-provider feature flags

`rig-core` 0.41's features are only about the HTTP stack and optional parsers. All 26 provider
modules compile unconditionally, so munibot can't forward per-provider features to rig to cut
compile time.

## No normalized stop reason

Neither `CompletionResponse<T>` nor `StreamingCompletionResponse<T>` exposes a finish reason at the
generic level. Only the provider-specific `raw_response: T` might carry one, which would defeat the
point of a provider-agnostic type. The adapter infers `StopReason` from content alone, so
`MaxTokens`, `StopSequence`, and `Refusal` are not distinguishable through it.

## No response headers in errors

`CompletionError` exposes a public `provider_response_status()` helper returning
`Option<http::StatusCode>`, which is enough to classify a failure as a rate limit, a transient server
error, or a permanent rejection. Nothing in rig's error model preserves response headers, though, so a
`Retry-After` value can never be extracted. This is why `AiError` has a `Rejected` variant distinct
from `Provider`: a 4xx will only ever fail the same way again, so it isn't worth retrying.
