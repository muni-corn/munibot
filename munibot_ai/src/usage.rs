//! Recording what a turn cost.
//!
//! The foundation for the usage dashboard and, later, rate limiting and spend
//! caps: every one of those reads from the same `ai_usage` rows this module
//! writes.

pub mod diesel_recorder;
pub mod recorder;

pub use diesel_recorder::DieselUsageRecorder;
pub use recorder::{UsageRecord, UsageRecorder};
