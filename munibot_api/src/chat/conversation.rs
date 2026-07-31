use serde::{Deserialize, Serialize};

/// One conversation in the sidebar. Deliberately slim -- just enough to
/// render and sort a list, the same philosophy as
/// [`crate::guilds::GuildSummary`].
///
/// `last_active_at` and `created_at` are pre-formatted (RFC 3339) rather than
/// a `chrono` type, since `chrono` is a server-only dependency of this crate
/// and these dtos have to compile for the wasm client too.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub persona_id: String,
    /// `None` until the first exchange gives it a title, or if title
    /// generation is turned off entirely.
    pub title: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub last_active_at: String,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::models::AiConversation> for ConversationSummary {
    fn from(row: munibot_core::db::models::AiConversation) -> Self {
        use chrono::DateTime;

        Self {
            id: row.id,
            persona_id: row.persona_id,
            title: row.title,
            archived: row.archived_at.is_some(),
            created_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.created_at,
                chrono::Utc,
            )
            .to_rfc3339(),
            last_active_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.last_active_at,
                chrono::Utc,
            )
            .to_rfc3339(),
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_core::db::models::AiConversation;

    use super::*;

    fn row() -> AiConversation {
        AiConversation {
            id: 1,
            platform: "web".to_string(),
            scope_key: "user:1".to_string(),
            persona_id: "companion".to_string(),
            owner_user_id: Some(1),
            title: Some("catching up".to_string()),
            summary: None,
            summary_tokens: 0,
            archived_at: None,
            created_at: NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
            last_active_at: NaiveDateTime::parse_from_str(
                "2026-07-30 10:05:00",
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap(),
        }
    }

    #[test]
    fn test_from_row_carries_the_title_and_persona() {
        let summary: ConversationSummary = row().into();
        assert_eq!(summary.id, 1);
        assert_eq!(summary.persona_id, "companion");
        assert_eq!(summary.title.as_deref(), Some("catching up"));
    }

    #[test]
    fn test_archived_is_derived_from_archived_at() {
        let mut archived_row = row();
        archived_row.archived_at = Some(
            NaiveDateTime::parse_from_str("2026-07-30 11:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        );
        let summary: ConversationSummary = archived_row.into();
        assert!(summary.archived);

        let summary: ConversationSummary = row().into();
        assert!(!summary.archived);
    }

    #[test]
    fn test_timestamps_render_as_rfc3339() {
        let summary: ConversationSummary = row().into();
        assert_eq!(summary.created_at, "2026-07-30T10:00:00+00:00");
        assert_eq!(summary.last_active_at, "2026-07-30T10:05:00+00:00");
    }
}
