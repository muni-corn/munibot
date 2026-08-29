//! Wire types for munibot's own chat page.
//!
//! Everything here has to compile for both `web` and `server`: the wasm
//! client only ever deserializes these, but the types themselves have no
//! reason to depend on anything native-only. Translating a `munibot_ai` or
//! `munibot_core` type into one of these is a `server`-only concern, kept in
//! each file's own `#[cfg(feature = "server")]`-gated `From` impl, since
//! `munibot_ai` and `munibot_core` are both server-only dependencies of this
//! crate.

mod attachment;
mod conversation;
mod error;
mod event;
mod memory;
mod message;
mod persona;
mod transcript;
mod usage;

pub use attachment::{ALLOWED_MEDIA_TYPES, AttachmentSummary, MAX_ATTACHMENT_BYTES};
pub use conversation::ConversationSummary;
pub use error::{ChatError, ChatResult};
pub use event::{ChatEvent, ChatFailureKind};
pub use memory::{MemoryEntry, MemorySettings};
pub use message::{ChatMessage, ChatRole};
pub use persona::PersonaSummary;
pub use transcript::{AiTranscript, TranscriptMessage, TranscriptToolCall};
pub use usage::{SpendCapStatus, UsageSummary, UsageTotals};
