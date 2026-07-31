use serde::{Deserialize, Serialize};

/// A person's memory opt-in setting.
///
/// A struct rather than a bare `bool` even though there is only one field
/// today: `ai_user_settings` is where a future setting (a retention window,
/// say) would land, and this dto should grow with it rather than the memory
/// panel's server functions switching shape later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct MemorySettings {
    pub opted_in: bool,
}

#[cfg(feature = "server")]
impl From<Option<munibot_core::db::models::AiUserSettings>> for MemorySettings {
    fn from(row: Option<munibot_core::db::models::AiUserSettings>) -> Self {
        Self {
            // no row at all means the same thing as an explicit false: memory
            // is opt-in, never assumed, and a person who has never touched
            // the setting has not opted in
            opted_in: row.is_some_and(|row| row.memory_opt_in),
        }
    }
}

/// One remembered fact, as shown in the memory panel.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::models::AiMemory> for MemoryEntry {
    fn from(row: munibot_core::db::models::AiMemory) -> Self {
        use chrono::DateTime;

        Self {
            key: row.key,
            value: row.value,
            updated_at: DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                row.updated_at,
                chrono::Utc,
            )
            .to_rfc3339(),
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_core::db::models::{AiMemory, AiUserSettings};

    use super::*;

    #[test]
    fn test_no_settings_row_means_not_opted_in() {
        let settings: MemorySettings = None.into();
        assert!(!settings.opted_in);
    }

    #[test]
    fn test_an_existing_row_carries_its_own_flag() {
        let now =
            NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let settings: MemorySettings = Some(AiUserSettings {
            user_id: 1,
            memory_opt_in: true,
            created_at: now,
            updated_at: now,
        })
        .into();
        assert!(settings.opted_in);
    }

    #[test]
    fn test_memory_entry_carries_the_key_and_value() {
        let now =
            NaiveDateTime::parse_from_str("2026-07-30 10:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let entry: MemoryEntry = AiMemory {
            id: 1,
            user_id: 1,
            key: "favorite_color".to_string(),
            value: "cyan".to_string(),
            created_at: now,
            updated_at: now,
        }
        .into();
        assert_eq!(entry.key, "favorite_color");
        assert_eq!(entry.value, "cyan");
        assert_eq!(entry.updated_at, "2026-07-30T10:00:00+00:00");
    }
}
