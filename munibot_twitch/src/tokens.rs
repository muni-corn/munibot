use async_trait::async_trait;
use twitch_irc::login::{StaticLoginCredentials, TokenStorage, UserAccessToken};
use twitch_oauth2::{AccessToken, UserToken};

#[derive(Debug)]
pub struct TwitchTokenStorage {
    pub user_access_token: UserAccessToken,
}

#[async_trait]
impl TokenStorage for TwitchTokenStorage {
    type LoadError = std::io::Error;
    type UpdateError = std::io::Error;

    async fn load_token(&mut self) -> Result<UserAccessToken, Self::LoadError> {
        Ok(self.user_access_token.clone())
    }

    async fn update_token(&mut self, token: &UserAccessToken) -> Result<(), Self::UpdateError> {
        token.clone_into(&mut self.user_access_token);
        Ok(())
    }
}

#[derive(Debug)]
pub struct TwitchAuth {
    access_token: UserToken,
    login_credentials: StaticLoginCredentials,
}

impl TwitchAuth {
    pub async fn new(login_name: &str, token: &str) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::new();
        let access_token = UserToken::from_token(&http_client, AccessToken::from(token)).await?;
        let login_credentials =
            StaticLoginCredentials::new(login_name.to_string(), Some(token.to_string()));

        Ok(Self {
            access_token,
            login_credentials,
        })
    }

    pub fn get_user_token(&self) -> &UserToken {
        &self.access_token
    }

    pub fn get_login_credentials(&self) -> &StaticLoginCredentials {
        &self.login_credentials
    }
}
