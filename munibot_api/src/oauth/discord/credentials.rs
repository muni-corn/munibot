//! Discord oauth client credentials, read from the environment.
use std::env::VarError;

/// The environment-derived values discord's oauth client needs: munibot's
/// own public base url (for building the redirect uri discord calls back
/// to) and the discord application's client id/secret.
pub struct Credentials {
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
}

impl Credentials {
    /// Reads `MUNIBOT_BASE_URL`, `DISCORD_APPLICATION_ID`, and
    /// `DISCORD_CLIENT_SECRET` from the environment.
    pub fn from_env() -> Result<Self, VarError> {
        Ok(Self {
            base_url: std::env::var("MUNIBOT_BASE_URL")?,
            client_id: std::env::var("DISCORD_APPLICATION_ID")?,
            client_secret: std::env::var("DISCORD_CLIENT_SECRET")?,
        })
    }
}
