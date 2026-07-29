//! Provider-neutral domain types for munibot's ai harness.
//!
//! Every other `munibot_ai_*` crate speaks these types. Nothing here performs
//! I/O, and nothing here depends on a specific model provider — that is what
//! makes the provider swappable.
//!
//! The one rule this crate exists to enforce: no provider's types ever leak
//! past `munibot_ai_provider`. If a type from `rig` or an HTTP client appears
//! in a signature outside that crate, the abstraction has been broken.

pub mod completion;
pub mod content;
pub mod message;
pub mod model;
pub mod stream;
pub mod tool;
pub mod usage;

pub use completion::{CompletionRequest, CompletionResponse, StopReason, ToolChoice};
pub use content::{ContentBlock, Image, ImageSource, Role};
pub use message::{History, Message, rough_token_estimate};
pub use model::{ModelParams, ModelRef, ModelRefError};
pub use stream::StreamEvent;
pub use tool::ToolSchema;
pub use usage::{Cost, Usage};
