use serde::{Deserialize, Serialize};

/// The largest attachment `upload_attachment` accepts, in bytes (checked
/// against the real, decoded size - never the base64 string's own length,
/// which runs about a third larger for nothing).
///
/// Shared with the composer (a future commit) so a person sees "too big"
/// before spending the round trip finding out, not just as a server-side
/// backstop.
pub const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

/// Every media type munibot can actually read as an image, matching what
/// `image::guess_format`-style sniffing and every provider this crate
/// targets both agree on. Anything else is rejected with a friendly reason
/// rather than uploaded and silently mishandled later.
pub const ALLOWED_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// A reference to one uploaded image - just enough for the browser to fetch
/// it directly (a future commit's `/attachments/{id}` route) and render a
/// thumbnail, never the bytes themselves.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AttachmentSummary {
    pub id: i64,
    pub media_type: String,
    pub byte_size: i32,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::models::AiAttachmentMeta> for AttachmentSummary {
    fn from(row: munibot_core::db::models::AiAttachmentMeta) -> Self {
        Self {
            id: row.id,
            media_type: row.media_type,
            byte_size: row.byte_size,
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::NaiveDateTime;
    use munibot_core::db::models::AiAttachmentMeta;

    use super::*;

    #[test]
    fn test_from_row_carries_id_media_type_and_size() {
        let row = AiAttachmentMeta {
            id: 1,
            conversation_id: 1,
            message_id: None,
            media_type: "image/png".to_string(),
            byte_size: 1234,
            sha256: "a".repeat(64),
            created_at: NaiveDateTime::parse_from_str("2026-08-07 10:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap(),
        };

        let summary: AttachmentSummary = row.into();
        assert_eq!(summary, AttachmentSummary {
            id: 1,
            media_type: "image/png".to_string(),
            byte_size: 1234,
        });
    }
}
