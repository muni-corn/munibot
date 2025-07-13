use std::{error::Error, fmt::Display};

use log::{debug, info};
use twitch_api::{
    HelixClient,
    helix::{ClientRequestError, channels::ChannelInformation, users::User},
    types::UserId,
};

use super::tokens::TwitchAuth;

pub struct TwitchAgent<'a> {
    helix_client: HelixClient<'a, reqwest::Client>,
    auth: TwitchAuth,
}

impl<'a> TwitchAgent<'a> {
    pub fn new(auth: TwitchAuth) -> Self {
        let helix_client = HelixClient::default();
        Self { helix_client, auth }
    }

    pub fn get_bot_id(&self) -> &UserId {
        &self.auth.get_user_token().user_id
    }

    pub fn get_helix_client(&self) -> &HelixClient<'a, reqwest::Client> {
        &self.helix_client
    }

    pub fn get_auth(&self) -> &TwitchAuth {
        &self.auth
    }

    /// Get the channel info for the given broadcaster ID
    pub async fn get_channel_info(
        &self,
        broadcaster_id: &str,
    ) -> Result<Option<ChannelInformation>, TwitchAgentError> {
        Ok(self
            .helix_client
            .get_channel_from_id(broadcaster_id, self.auth.get_user_token())
            .await?)
    }

    pub async fn get_user_from_login(&self, login: &str) -> Result<Option<User>, TwitchAgentError> {
        Ok(self
            .helix_client
            .get_user_from_login(login, self.auth.get_user_token())
            .await?)
    }

    pub async fn ban_user(
        &self,
        ban_user_id: &UserId,
        reason: &str,
        broadcaster_id: &UserId,
    ) -> Result<(), TwitchAgentError> {
        debug!("attempting to ban user {}", ban_user_id);
        let moderator_id = self.get_bot_id();
        self.helix_client
            .ban_user(
                ban_user_id,
                reason,
                None,
                broadcaster_id,
                moderator_id,
                self.auth.get_user_token(),
            )
            .await?;
        info!("munibot banned user {ban_user_id} from broadcaster {broadcaster_id}");
        Ok(())
    }
}

#[derive(Debug)]
pub enum TwitchAgentError {
    CredentialsError(String),
    MissingCredentials,
    ReqwestError(reqwest::Error),
    HelixRequestError(ClientRequestError<reqwest::Error>),
    Other(String),
}

impl From<reqwest::Error> for TwitchAgentError {
    fn from(e: reqwest::Error) -> Self {
        Self::ReqwestError(e)
    }
}

impl From<ClientRequestError<reqwest::Error>> for TwitchAgentError {
    fn from(e: ClientRequestError<reqwest::Error>) -> Self {
        Self::HelixRequestError(e)
    }
}

impl Display for TwitchAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TwitchAgentError::CredentialsError(e) => {
                write!(f, "error with twitch credentials: {e}")
            }
            TwitchAgentError::MissingCredentials => write!(f, "twitch credentials went missing"),
            TwitchAgentError::ReqwestError(e) => write!(f, "twitch agent request error: {e}"),
            TwitchAgentError::Other(e) => write!(f, "twitch agent error: {e}"),
            TwitchAgentError::HelixRequestError(e) => write!(f, "helix client threw an error: {e}"),
        }
    }
}

impl Error for TwitchAgentError {}
