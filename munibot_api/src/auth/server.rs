use async_trait::async_trait;
use axum_session_auth::{Authentication, HasPermission};
use axum_session_redispool::SessionRedisPool;
use munibot_core::db::{self, DbPool, operations};
use serde::{Deserialize, Serialize};

use crate::auth::UserData;

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

impl From<db::models::User> for User {
    fn from(row: db::models::User) -> Self {
        Self {
            id: row.id,
            data: UserData {
                display_name: row.display_name,
                avatar_url: row.avatar_url,
            },
            permissions: HashSet::new(),
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

        let permissions = operations::list_user_permissions(pool, user_id)
            .await?
            .into_iter()
            .collect();

        let mut user: User = row.into();
        user.permissions = permissions;
        Ok(user)
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
    async fn has(&self, perm: &str, _pool: &Option<&DbPool>) -> bool {
        self.permissions.contains(perm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_with(permissions: &[&str]) -> User {
        User {
            id: 1,
            data: UserData {
                display_name: "muni".to_string(),
                avatar_url: None,
            },
            permissions: permissions.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn test_has_matches_a_granted_permission() {
        let user = user_with(&["operator"]);
        assert!(futures::executor::block_on(user.has("operator", &None)));
    }

    #[test]
    fn test_has_does_not_match_an_ungranted_permission() {
        let user = user_with(&[]);
        assert!(!futures::executor::block_on(user.has("operator", &None)));
    }

    #[test]
    fn test_converting_from_a_db_row_starts_with_no_permissions() {
        let row = munibot_core::db::models::User {
            id: 1,
            display_name: "muni".to_string(),
            avatar_url: None,
            created_at: chrono::Utc::now().naive_utc(),
        };
        let user: User = row.into();
        assert!(user.permissions.is_empty());
    }
}
