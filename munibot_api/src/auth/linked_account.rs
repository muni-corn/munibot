use serde::{Deserialize, Serialize};

/// One provider account linked to the signed-in user, as shown on an
/// account settings page.
///
/// Deliberately carries none of `munibot_core::db::models::LinkedAccount`'s
/// own token fields - see `docs/notes/gui-configuration-research.md`'s own
/// note on that model mixing display-safe and sensitive fields together;
/// this dto is the compile-time separation that model itself doesn't
/// enforce, so a future field added there can never accidentally reach the
/// wasm client just by deriving `From` carelessly.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LinkedAccountSummary {
    pub provider: String,
    pub username: String,
    pub linked_at: String,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::models::LinkedAccount> for LinkedAccountSummary {
    fn from(row: munibot_core::db::models::LinkedAccount) -> Self {
        use chrono::DateTime;

        Self {
            provider: row.provider,
            username: row.username,
            linked_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.created_at,
                chrono::Utc,
            )
            .to_rfc3339(),
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_core::db::models::LinkedAccount;

    use super::*;

    #[test]
    fn test_from_linked_account_excludes_every_token_field() {
        let now =
            NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let summary: LinkedAccountSummary = LinkedAccount {
            id: 1,
            user_id: 1,
            provider: "github".to_string(),
            provider_user_id: "12345".to_string(),
            username: "octocat".to_string(),
            access_token: "super-secret-token".to_string(),
            refresh_token: Some("super-secret-refresh".to_string()),
            token_expires_at: None,
            created_at: now,
            updated_at: now,
        }
        .into();

        assert_eq!(summary.provider, "github");
        assert_eq!(summary.username, "octocat");
        assert_eq!(summary.linked_at, "2026-07-30T10:00:00+00:00");
        // the point of this dto's existence: no field here can ever hold a
        // token, so there is nothing left to assert isn't leaking one
    }
}
