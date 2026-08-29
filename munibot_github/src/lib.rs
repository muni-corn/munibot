//! GitHub App integration for munibot's autonomous pipeline.
//!
//! Implements `munibot_vcs`'s forge-agnostic traits over
//! [`octocrab`](https://docs.rs/octocrab), authenticating as a GitHub App
//! installation rather than a personal access token -- see
//! `docs/plans/ai/milestone-5-autonomous.md` phase 20 for the full design.

mod auth;
mod error;
mod forge;
mod webhook;

pub use auth::{InstallationTokenCache, OctocrabTokenMinter, TokenMinter};
pub use error::GitHubError;
pub use forge::GitHubForge;
pub use webhook::{normalize_webhook, verify_signature};
