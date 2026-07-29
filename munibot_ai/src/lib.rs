//! munibot's ai agent harness.
//!
//! A provider-agnostic, tool-using, multi-persona AI system serving casual
//! conversation, emotional support, creative writing, deep research, and
//! autonomous software development.
//!
//! See `docs/plans/ai/overview.md` for the full architecture and
//! `docs/notes/ai-preflight-findings.md` for the provider API research this
//! crate is built on.
//!
//! Organized as modules rather than separate crates for a project this size:
//! `types` holds the provider-neutral domain types everything else speaks, and
//! `provider` is the only module allowed to depend on `rig-core`. Forge
//! integration (`munibot_vcs`, `munibot_github`) and the in-container
//! tool agent (`munibot_toolagent`) stay as separate crates, since neither
//! belongs to "ai" as a concern and the tool agent must stay small enough to
//! ship into an untrusted container.

pub mod provider;
pub mod types;

pub use types::{
    AiError, CompletionRequest, CompletionResponse, ContentBlock, Cost, History, Image,
    ImageSource, Message, ModelParams, ModelRef, ModelRefError, Role, StopReason, StreamEvent,
    ToolChoice, ToolSchema, Usage, rough_token_estimate,
};
