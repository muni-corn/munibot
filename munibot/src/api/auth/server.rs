use async_trait::async_trait;
use axum_session_auth::{Authentication, HasPermission};
use axum_session_redispool::SessionRedisPool;
use munibot_core::db::{DbPool, operations};
use serde::{Deserialize, Serialize};

use crate::api::auth::UserData;

/// Alias for the session type this app uses everywhere: `String` session
/// IDs, redis-backed session storage, and a real diesel pool for loading the
/// current user.
pub type AuthSession = axum_session_auth::AuthSession<User, String, SessionRedisPool, DbPool>;

/// The session's notion of the signed-in user.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct User {
    pub id: i64,

    #[serde(flatten)]
    pub data: UserData,
}

impl From<munibot_core::db::models::User> for User {
    fn from(row: munibot_core::db::models::User) -> Self {
        Self {
            id: row.id,
            data: UserData {
                display_name: row.display_name,
                avatar_url: row.avatar_url,
            },
        }
    }
}

#[async_trait]
impl Authentication<User, String, DbPool> for User {
    async fn load_user(id: String, pool: Option<&DbPool>) -> Result<User, anyhow::Error> {
        let pool = pool.ok_or_else(|| anyhow::anyhow!("no db pool available"))?;
        let user_id: i64 = id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid user id in session '{id}': {e}"))?;

        let row = operations::get_user(pool, user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no user found with id '{id}'"))?;

        Ok(row.into())
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
impl HasPermission<DbPool> for User {
    // munibot doesn't have a per-user permission system yet, so nothing ever
    // matches. a future `BotAdmin`-style flag can replace this.
    async fn has(&self, _perm: &str, _pool: &Option<&DbPool>) -> bool {
        false
    }
}
