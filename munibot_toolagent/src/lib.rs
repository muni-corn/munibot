//! The in-container rpc server executing filesystem and shell tools for the
//! ai sandbox (`munibot_ai::sandbox`).
//!
//! Split into a library and a thin binary (`main.rs`) rather than one big
//! `main.rs`, so the wire protocol and its tests exist independently of
//! whatever `main` currently wires up -- see `docs/plans/ai/milestone-4
//! -sandbox.md` phase 18 for the commit-by-commit build order this follows.
//!
//! Takes no munibot dependency at all -- see this crate's `Cargo.toml` for
//! why.

pub mod protocol;
