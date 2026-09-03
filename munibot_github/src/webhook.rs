//! Everything to do with an inbound GitHub webhook delivery: verifying it
//! actually came from GitHub, then normalizing it into a
//! `munibot_vcs::ForgeEvent`.

mod installation;
mod normalize;
mod signature;

pub use installation::{InstallationEvent, InstalledRepositoryRef, parse_installation_event};
pub use normalize::{issue_text, normalize_webhook};
pub use signature::verify_signature;
