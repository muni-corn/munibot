//! munibot's conversation interface.
//!
//! A provider-agnostic way to hold a plain, temporary conversation with a
//! model: [`Chat`] streams a reply to whatever history it is handed and
//! remembers nothing afterwards.
//!
//! Organized as modules rather than separate crates for a project this size:
//! `types` holds the provider-neutral domain types everything else speaks, and
//! `provider` is the only module allowed to depend on `rig-core`.

pub mod config;
pub mod conversation;
pub mod provider;
pub mod types;

pub use service::{Ai, AiTurnRequest};
pub use types::{
    AiError, CompletionRequest, CompletionResponse, ContentBlock, Cost, History, Image,
    ImageSource, Message, ModelParams, ModelRef, ModelRefError, Role, StopReason, StreamEvent,
    ToolChoice, ToolSchema, Usage, rough_token_estimate,
};
