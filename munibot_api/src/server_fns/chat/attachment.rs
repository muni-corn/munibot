use dioxus::prelude::*;

use crate::chat::{AttachmentSummary, ChatResult};

/// Uploads one image to be attached to a message the caller sends shortly
/// after, returning a reference the composer can preview and later pass to
/// `send_message`'s own `attachment_ids`.
///
/// Split from `send_message` deliberately, the same reasoning as splitting
/// streaming from it: a person picks or pastes an image before they've
/// necessarily finished typing, and an upload can fail (too big, wrong
/// type) independently of whether the message itself ever gets sent.
///
/// `data_base64` arrives as a plain string, not a `multipart/form-data`
/// body, since dioxus server functions serialize their arguments as JSON --
/// there is no multipart plumbing to hook into without a second, bespoke
/// HTTP route.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn upload_attachment(
    conversation_id: i64,
    media_type: String,
    data_base64: String,
) -> ChatResult<AttachmentSummary> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use munibot_core::db::{models::NewAiAttachment, operations::ai};
    use sha2::{Digest, Sha256};

    use crate::{
        chat::{ALLOWED_MEDIA_TYPES, ChatError, MAX_ATTACHMENT_BYTES},
        server_fns::chat::conversation::owned_conversation,
    };

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    owned_conversation(&pool, conversation_id, user.id).await?;

    if !ALLOWED_MEDIA_TYPES.contains(&media_type.as_str()) {
        return Err(ChatError::AttachmentRejected(format!(
            "'{media_type}' isn't a supported image type -- try png, jpeg, gif, or webp"
        )));
    }

    let data = STANDARD.decode(data_base64.as_bytes()).map_err(|e| {
        ChatError::AttachmentRejected(format!("that upload wasn't valid image data :< {e}"))
    })?;

    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err(ChatError::AttachmentRejected(format!(
            "that image is {} bytes, over the {} byte limit -- try a smaller one",
            data.len(),
            MAX_ATTACHMENT_BYTES
        )));
    }
    let byte_size = i32::try_from(data.len())
        .map_err(|_| ChatError::AttachmentRejected("that image is too big".to_string()))?;

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let sha256 = hex_encode(&hasher.finalize());

    let row = ai::create_attachment(&pool, NewAiAttachment {
        conversation_id,
        media_type,
        byte_size,
        sha256,
        data,
        created_at: chrono::Utc::now().naive_utc(),
    })
    .await?;

    Ok(row.into())
}

/// Renders bytes as lowercase hex, for `sha256` -- not worth a dependency
/// on `hex` for one call site.
#[cfg(feature = "server")]
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    #[test]
    fn test_hex_encode_matches_known_sha256_of_empty_input() {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(b"");
        let hash = hasher.finalize();

        // the well-known SHA-256 of the empty string, as a sanity check
        // that byte order and case come out the way every other tool
        // expects
        assert_eq!(
            hex_encode(&hash),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
