//! Provider moderation: an inbound-and-outbound content check, layered on
//! top of everything else in this crate that already screens a turn.
//!
//! Distinct from [`crate::abuse`] and [`crate::crisis`]: those work from
//! munibot's own heuristics (a signature list, a recent-activity tracker, a
//! cheap classifier call), while this defers to the model provider's own
//! moderation endpoint - where one actually exists, since not every
//! provider ships one (see [`OpenAiModerator`] for the one that does).
//!
//! Every check reads [`ModerationPolicy`] from the *persona* being run, not
//! a global setting - a casual chat persona wants a moderation outage to
//! never silence it, while a persona reaching for
//! [`crate::tools::RiskTier::Privileged`] tools wants the opposite: refuse
//! outright rather than let a real-world action through unchecked because
//! the check itself couldn't run.

mod gate;
mod moderator;
mod openai;
mod policy;

pub use gate::ModerationGate;
pub use moderator::{ModerationVerdict, Moderator};
pub use openai::OpenAiModerator;
pub use policy::ModerationPolicy;
