//! Conversation history and long-term memory.
//!
//! Two [`SessionStore`] implementations live here and are not
//! interchangeable in intent: [`InMemorySessionStore`] is what every unit test
//! uses and what a keyless development boot falls back to, while
//! [`DieselSessionStore`] is the production store that makes a conversation
//! survive a restart.
//!
//! Per-user opt-in memory, as distinct from conversation history, is still to
//! come.

pub mod context;
pub mod conversation;
pub mod diesel_store;
pub mod in_memory;
pub mod store;

pub use context::assemble_context;
pub use conversation::{Conversation, ConversationScope};
pub use diesel_store::DieselSessionStore;
pub use in_memory::InMemorySessionStore;
pub use store::SessionStore;
