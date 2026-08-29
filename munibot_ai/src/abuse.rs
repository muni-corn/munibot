//! Abuse detection and escalating cooldowns.
//!
//! `crate::limits` bounds *cost*: how many requests, tokens, or dollars a
//! scope may spend. This module bounds *behaviour* instead - the same
//! signed-in stranger a public web chat has no invite gate or channel gate
//! against (see the milestone 6 plan's own framing) farming free tokens
//! with repeated near-identical prompts, probing with well-known
//! prompt-injection phrasings, or rapidly switching personas to see what
//! sticks. None of that is necessarily expensive enough to trip a rate
//! limit or spend cap, but all of it is worth an escalating cooldown - and
//! worth logging every single trip, since a heuristic this cheap will have
//! false positives an operator needs to be able to see and tune away
//! rather than have silently dropped.

mod cooldown;
mod detector;
mod error;
mod signature;
mod store;
mod thresholds;
mod tracker;

pub use cooldown::CooldownPolicy;
pub use detector::{AbuseDetector, AbuseSignal};
pub use error::AbuseError;
pub use signature::injection_signature;
pub use store::{AbuseCooldownRow, AbuseStore, DieselAbuseStore};
pub use thresholds::DetectionThresholds;
