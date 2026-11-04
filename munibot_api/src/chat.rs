//! Wire types for munibot's chat page.
//!
//! Everything here has to compile for both `web` and `server`: the wasm
//! client builds and sends conversations, and the server answers them.
//! Translating a `munibot_ai` type into one of these is a `server`-only
//! concern, kept in each file's own `#[cfg(feature = "server")]`-gated
//! impl, since `munibot_ai` is a server-only dependency of this crate.

mod error;
mod event;
mod message;
mod validate;

pub use attachment::{ALLOWED_MEDIA_TYPES, AttachmentSummary, MAX_ATTACHMENT_BYTES};
pub use conversation::ConversationSummary;
pub use error::{ChatError, ChatResult};
pub use event::{ChatEvent, ChatFailureKind};
pub use memory::{MemoryEntry, MemorySettings};
pub use message::{ChatMessage, ChatRole};
pub use persona::PersonaSummary;
pub use transcript::{AiTranscript, TranscriptMessage, TranscriptToolCall};
pub use usage::{SpendCapStatus, UsageSummary, UsageTotals};
