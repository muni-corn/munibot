//! Database operations for `ai_attachments`.
//!
//! In its own submodule for the same reason `limits` and `usage` are: this
//! crate's own `ai.rs` is already long, and a fourth self-contained concern
//! is a new file, not more lines in that one.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::db::{
    DbPool,
    models::{AiAttachment, AiAttachmentMeta, NewAiAttachment},
    schema::ai_attachments,
};

diesel::define_sql_function!(fn last_insert_id() -> diesel::sql_types::Unsigned<diesel::sql_types::Bigint>);

/// Stores a freshly uploaded attachment, returning its metadata (not its own
/// bytes back - the caller already has those, it just sent them).
///
/// `message_id` starts `NULL`: an upload always happens before the message
/// that will reference it exists, since SSE can't carry a pasted image as a
/// query string. See [`link_attachment_to_message`] for the other half.
pub async fn create_attachment(
    pool: &DbPool,
    attachment: NewAiAttachment,
) -> QueryResult<AiAttachmentMeta> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::insert_into(ai_attachments::table)
        .values(&attachment)
        .execute(&mut conn)
        .await?;

    let id: u64 = diesel::select(last_insert_id())
        .get_result(&mut conn)
        .await?;
    ai_attachments::table
        .find(id as i64)
        .select(AiAttachmentMeta::as_select())
        .first(&mut conn)
        .await
}

/// Links an already-uploaded attachment to the message that ended up
/// referencing it, once that message is actually persisted.
pub async fn link_attachment_to_message(
    pool: &DbPool,
    attachment_id: i64,
    message_id: i64,
) -> QueryResult<()> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    diesel::update(ai_attachments::table.find(attachment_id))
        .set(ai_attachments::message_id.eq(message_id))
        .execute(&mut conn)
        .await?;
    Ok(())
}

/// An attachment's metadata, without its own bytes - for a thumbnail list
/// or a history reconstruction pass that only needs to know one exists.
pub async fn get_attachment_meta(pool: &DbPool, id: i64) -> QueryResult<Option<AiAttachmentMeta>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_attachments::table
        .find(id)
        .select(AiAttachmentMeta::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// An attachment's full row, bytes included - for actually serving it (over
/// HTTP, or into a provider request's own base64 encoding).
pub async fn get_attachment(pool: &DbPool, id: i64) -> QueryResult<Option<AiAttachment>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_attachments::table
        .find(id)
        .select(AiAttachment::as_select())
        .first(&mut conn)
        .await
        .optional()
}

/// Every attachment linked to one message, metadata only - what history
/// reconstruction and the message list both read to know an attachment
/// exists at all, before fetching any one attachment's own bytes.
pub async fn list_attachments_for_message(
    pool: &DbPool,
    message_id: i64,
) -> QueryResult<Vec<AiAttachmentMeta>> {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    ai_attachments::table
        .filter(ai_attachments::message_id.eq(message_id))
        .select(AiAttachmentMeta::as_select())
        .load(&mut conn)
        .await
}
