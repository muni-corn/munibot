//! Provider-neutral domain types for munibot's ai harness.
//!
//! Every other module in this crate speaks these types. Nothing here performs
//! I/O, and nothing here depends on a specific model provider — that is what
//! makes the provider swappable.
//!
//! The one rule this module exists to enforce: no provider's types ever leak
//! past the `provider` module. If a type from `rig` or an HTTP client appears
//! in a signature outside it, the abstraction has been broken.

pub mod completion;
pub mod content;
pub mod error;
pub mod message;
pub mod model;
pub mod stream;
pub mod tool;
pub mod usage;

pub use completion::{CompletionRequest, CompletionResponse, StopReason, ToolChoice};
pub use content::{ContentBlock, Image, ImageSource, Role};
pub use error::AiError;
pub use message::{History, Message, rough_token_estimate};
pub use model::{ModelParams, ModelRef, ModelRefError};
pub use stream::StreamEvent;
pub use tool::ToolSchema;
pub use usage::{Cost, Usage};
