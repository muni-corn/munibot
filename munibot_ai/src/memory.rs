//! Conversation history and long-term memory.
//!
//! Stays in-memory-only through this milestone - the diesel-backed store, and
//! per-user opt-in memory beyond conversation history, both arrive in milestone
//! 2. Nothing here depends on `munibot_core` yet, so this module carries no
//! migrations and no database dependency.

pub mod conversation;
pub mod store;

pub use conversation::{Conversation, ConversationScope};
pub use store::SessionStore;
