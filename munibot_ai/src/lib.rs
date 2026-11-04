//! munibot's conversation interface.
//!
//! A provider-agnostic way to hold a plain, temporary conversation with a
//! model: [`Chat`] streams a reply to whatever history it is handed and
//! remembers nothing afterwards.
//!
//! Organized as modules rather than separate crates for a project this size:
//! `types` holds the provider-neutral domain types everything else speaks, and
//! `provider` is the only module allowed to depend on `rig-core`. Forge
//! integration (`munibot_vcs`, `munibot_github`) and the in-container
//! tool agent (`munibot_toolagent`) stay as separate crates, since neither
//! belongs to "ai" as a concern and the tool agent must stay small enough to
//! ship into an untrusted container.

pub mod audit;
pub mod crisis;
pub mod harness;
pub mod limits;
pub mod memory;
pub mod persona;
pub mod pipeline;
pub mod provider;
pub mod sandbox;
pub mod service;
pub mod tools;
pub mod types;
pub mod usage;

pub use service::{Ai, AiTurnRequest};
pub use types::{
    AiError, CompletionRequest, CompletionResponse, ContentBlock, Cost, History, Image,
    ImageSource, Message, ModelParams, ModelRef, ModelRefError, Role, StopReason, StreamEvent,
    ToolChoice, ToolSchema, Usage, rough_token_estimate,
};
