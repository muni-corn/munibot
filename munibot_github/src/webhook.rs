//! Everything to do with an inbound GitHub webhook delivery: verifying it
//! actually came from GitHub, then normalizing it into a
//! `munibot_vcs::ForgeEvent`.

mod signature;

pub use signature::verify_signature;
