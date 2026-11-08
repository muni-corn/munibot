//! Wire types for munibot's chat page.
//!
//! Everything here has to compile for both `web` and `server`: the wasm
//! client builds and sends conversations, and the server answers them.
//! Translating between these and rig's types (re-exported as
//! `munibot_ai::rig`) is a `server`-only concern, kept in each file's own
//! `#[cfg(feature = "server")]`-gated impl, since `munibot_ai` is a
//! server-only dependency of this crate.
//!
//! These are deliberately not rig's own message and stream types. The
//! browser is untrusted, so what it may send is narrower than a rig
//! `Message` (which can carry system prompts, tool results, and images), and
//! the wasm bundle shouldn't have to compile rig at all.

mod error;
mod event;
#[cfg(feature = "server")]
mod failure;
mod message;
mod validate;

pub use error::{ChatError, ChatResult};
pub use event::ChatEvent;
pub use message::{ChatMessage, ChatRole};
pub use validate::{MAX_CONVERSATION_CHARS, MAX_CONVERSATION_MESSAGES, validate_conversation};
