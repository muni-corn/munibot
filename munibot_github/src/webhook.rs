//! Everything to do with an inbound GitHub webhook delivery: verifying it
//! actually came from GitHub, then normalizing it into a
//! `munibot_vcs::ForgeEvent`.

mod normalize;
mod signature;

pub use normalize::normalize_webhook;
pub use signature::verify_signature;
