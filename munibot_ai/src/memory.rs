//! Conversation history and long-term memory.
//!
//! Two [`SessionStore`] implementations live here and are not
//! interchangeable in intent: [`InMemorySessionStore`] is what every unit test
//! uses and what a keyless development boot falls back to, while
//! [`DieselSessionStore`] is the production store that makes a conversation
//! survive a restart.
//!
//! [`MemoryStore`] is a different concept entirely, despite the name overlap
//! with this module: it is a person's own opt-in, cross-conversation memory,
//! not a single conversation's history. Opt-in gating is layered on top of it
//! separately rather than baked in - see the type's own doc comment.

pub mod context;
pub mod conversation;
pub mod diesel_memory_store;
pub mod diesel_opt_in;
pub mod diesel_store;
pub mod directory;
pub mod in_memory;
pub mod opt_in;
pub mod store;
pub mod summarise;
pub mod user_memory;

pub use context::assemble_context;
pub use conversation::{Conversation, ConversationScope};
pub use diesel_memory_store::DieselMemoryStore;
pub use diesel_opt_in::DieselMemoryOptIn;
pub use diesel_store::DieselSessionStore;
pub use directory::{ConversationDirectory, ConversationEntry};
pub use in_memory::InMemorySessionStore;
pub use opt_in::{GatedMemoryStore, MemoryOptIn};
pub use store::SessionStore;
pub use summarise::{CompactionPersona, CompactionSettings, Summariser, compact_if_needed};
pub use user_memory::{Memory, MemoryStore};
