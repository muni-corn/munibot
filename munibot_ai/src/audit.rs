//! Auditing individual tool calls.
//!
//! The only way to debug a bad tool loop after the fact, and what a chat
//! surface's tool activity display can read back for a past conversation.

pub mod diesel_auditor;
pub mod record;

pub use diesel_auditor::DieselToolAuditor;
pub use record::{ToolAuditor, ToolCallRecord, ToolCallStatus};
