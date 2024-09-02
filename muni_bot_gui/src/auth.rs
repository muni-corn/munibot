use std::collections::{HashSet, VecDeque};

use leptos::*;
use log::debug;
use serde::{Deserialize, Serialize};
use strum::EnumString;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, EnumString)]
pub enum Permissions {
    #[strum(to_string = "muni_bot::admin")]
    BotAdmin,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserData {
    pub discord_id: Option<u64>,
    pub twitch_id: Option<u64>,
    pub discord_token: Option<String>,
    pub twitch_token: Option<String>,
    pub permissions: HashSet<Permissions>,
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use std::str::FromStr;

    use async_trait::async_trait;
    use axum_session_auth::{Authentication, HasPermission};
    use axum_session_surreal::SessionSurrealPool;
    use leptos::{use_context, ServerFnError};
    use log::debug;
    use serde::{Deserialize, Serialize};
    use surrealdb::{engine::remote::ws, sql::Thing};
    use thiserror::Error;

    use super::{Permissions, UserData};

    type Sess = SessionSurrealPool<ws::Client>;
    type Pool = surrealdb::Surreal<ws::Client>;
    pub type AuthSession = axum_session_auth::AuthSession<User, String, Sess, Pool>;

    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
    pub struct User {
        pub id: Thing,

        #[serde(flatten)]
        pub data: UserData,
    }

    pub fn pool() -> Result<Pool, ServerFnError> {
        use_context::<Pool>().ok_or_else(|| {
            ServerFnError::ServerError("db pool was requested, but there is none".into())
        })
    }

    pub fn auth() -> Result<AuthSession, ServerFnError> {
        use_context::<AuthSession>().ok_or_else(|| {
            ServerFnError::ServerError("auth session requested, but there is none".into())
        })
    }

    #[async_trait]
    impl Authentication<User, String, Pool> for User {
        async fn load_user(id: String, pool: Option<&Pool>) -> Result<User, anyhow::Error> {
            match pool {
                Some(pool) => {
                    if let Some(user) = pool
                        .query("SELECT * FROM users WHERE id = $id")
                        .bind(("id", id))
                        .await?
                        .take(0)?
                    {
                        return Ok(user);
                    } else {
                        return Err(anyhow::anyhow!("no user found"));
                    }
                }
                None => Err(anyhow::anyhow!("no pool to load user from")),
            }
        }

        fn is_authenticated(&self) -> bool {
            true
        }

        fn is_active(&self) -> bool {
            true
        }

        fn is_anonymous(&self) -> bool {
            false
        }
    }

    #[async_trait]
    impl HasPermission<Pool> for User {
        async fn has(&self, perm: &str, _pool: &Option<&Pool>) -> bool {
            let Ok(permission) = Permissions::from_str(perm) else {
                return false;
            };
            self.data.permissions.contains(&permission)
        }
    }

    pub async fn exchange_code_for_token(
        discord_code: &str,
    ) -> Result<String, DiscordAuthCodeGrantError> {
        let client = reqwest::Client::new();
        let res = client
            .post("https://discord.com/api/oauth2/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", discord_code),
                ("redirect_uri", "http://localhost:3000/discord"),
            ])
            .basic_auth(
                std::env::var("DISCORD_CLIENT_ID").unwrap(),
                Some(std::env::var("DISCORD_CLIENT_SECRET").unwrap()),
            )
            .send()
            .await?
            .json::<AuthorizationCodeGrantResponse>()
            .await?;

        match res {
            AuthorizationCodeGrantResponse::Success { access_token, .. } => Ok(access_token),
            AuthorizationCodeGrantResponse::Error {
                error,
                error_description,
            } => Err(DiscordAuthCodeGrantError::DiscordError {
                error,
                error_description,
            }),
        }
    }

    pub async fn get_discord_user_id(token: &str) -> Result<u64, DiscordAuthCodeGrantError> {
        let client = reqwest::Client::new();
        let res = client
            .get("https://discord.com/api/users/@me")
            .bearer_auth(token)
            .send()
            .await?
            .json::<discord::model::User>()
            .await?;

        Ok(res.id.0)
    }

    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    pub enum AuthorizationCodeGrantResponse {
        Error {
            error: String,
            error_description: Option<String>,
        },

        #[allow(dead_code)]
        Success {
            access_token: String,
            token_type: String,
            expires_in: u64,
            refresh_token: String,
            scope: String,
        },
    }

    #[derive(Debug, Error)]
    pub enum DiscordAuthCodeGrantError {
        #[error("reqwest error: {0}")]
        Reqwest(#[from] reqwest::Error),
        #[error("error from discord: {error} ({error_description:?})")]
        DiscordError {
            error: String,
            error_description: Option<String>,
        },
    }
}

#[server(Login, "/api")]
pub async fn login_discord(discord_code: String) -> Result<(), ServerFnError> {
    use self::ssr::*;
    let pool = pool()?;
    let auth = auth()?;

    // get discord user ID using code
    debug!("exchanging code for token");
    let user_token = exchange_code_for_token(&discord_code).await?;
    debug!("getting current user id");
    let discord_user_id = get_discord_user_id(&user_token).await?;

    // get or create muni_bot user from discord ID
    debug!(
        "getting or creating user from discord ID: {}",
        discord_user_id
    );
    let gui_user: User = if let Some(user) = pool
        .query("SELECT * FROM users WHERE discord_id = $discord_id")
        .bind(("discord_id", discord_user_id))
        .await?
        .take(0)?
    {
        user
    } else if let Some(new_user) = {
        debug!("creating new user");
        VecDeque::from(
            pool.insert("users")
                .content(UserData {
                    discord_id: Some(discord_user_id),
                    twitch_id: None,
                    discord_token: None,
                    twitch_token: None,
                    permissions: HashSet::new(),
                })
                .await?,
        )
        .pop_front()
    } {
        debug!("returning existing user");
        new_user
    } else {
        return Err(ServerFnError::new(
            "couldn't create a new user with discord id",
        ));
    };

    auth.login_user(gui_user.id.to_string());
    auth.remember_user(false);
    leptos_axum::redirect("/dashboard");

    Ok(())
}

#[server(Logout, "/api")]
pub async fn logout() -> Result<(), ServerFnError> {
    use self::ssr::*;

    let auth = auth()?;

    auth.logout_user();
    leptos_axum::redirect("/");

    Ok(())
}
