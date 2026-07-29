//! Provider-agnostic model access for munibot's ai harness.
//!
//! `rig-core` is a dependency of this module and no other. Every other module
//! in this crate speaks [`crate::types`] and the `Provider` trait defined here,
//! never a rig type directly.
//!
//! `rig-core`'s `CompletionModel` trait is not object-safe (it carries
//! associated types, a `Clone` supertrait, and `impl Future` return positions),
//! and 0.41 has no runtime provider-selection helper to lean on. This module's
//! job is to hide both of those facts behind an object-safe trait, so the rest
//! of the harness never has to care. See `docs/notes/ai-preflight-findings.md`
//! for how that was verified.
