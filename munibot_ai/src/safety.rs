//! Safety event auditing: one durable record of every trip of a rate
//! limit, a spend cap, a moderation check, or a crisis classifier.
//!
//! Distinct from [`crate::abuse`]'s own `ai_abuse_cooldowns` table (which
//! already exists purely to hold *state* an escalating cooldown needs) and
//! from [`crate::audit::ToolAuditor`] (which records what a tool call did,
//! not a safety refusal) - this exists purely so an operator can tune
//! every safety system with real numbers, without any of it becoming a
//! surveillance log: see [`SafetyEvent`]'s own doc comment for why raw
//! content never appears here.

mod diesel_auditor;
mod event;

pub use diesel_auditor::DieselSafetyEventAuditor;
pub use event::{SafetyEvent, SafetyEventAuditor, SafetyEventType, hash_content};
