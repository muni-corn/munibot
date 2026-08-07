// attachments.rs: serves one uploaded attachment's own bytes directly.
//
// A dioxus server function's response is always JSON, so it can't hand a
// browser a real `Content-Type: image/png` an `<img>` tag can just point
// at -- this is a plain axum route for exactly that reason, the same as
// `munibot_api::oauth::routes` is for sign-in/logout.

use axum::{
    Router,
    body::Body,
    extract::{Extension, Path},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use munibot_api::auth::server::AuthSession;
use munibot_core::db::{DbPool, operations::ai};

/// The one route this module adds: `GET /attachments/{id}`.
pub fn router() -> Router {
    Router::new().route("/attachments/{id}", get(serve_attachment))
}

/// Streams one attachment's bytes back, if the signed-in caller actually
/// owns the conversation it belongs to.
///
/// Returns the same 404 whether the attachment doesn't exist or belongs to
/// someone else -- the same reasoning `munibot_api`'s own conversation
/// ownership check already applies (see `ChatError::NotYourConversation`'s
/// doc comment): a caller must never be able to tell "doesn't exist" and
/// "not yours" apart by probing ids.
///
/// A channel-scoped conversation (Discord, Twitch) has no
/// `owner_user_id` at all, so its attachments -- none exist yet, since
/// only the web composer can upload one -- would 404 for everyone here
/// regardless of who asks, which is the correct default rather than a
/// gap: nobody has an identity to own them by, over this route.
async fn serve_attachment(
    auth: AuthSession,
    Path(id): Path<i64>,
    Extension(pool): Extension<DbPool>,
) -> Result<impl IntoResponse, StatusCode> {
    let user = auth.current_user.ok_or(StatusCode::UNAUTHORIZED)?;

    let attachment = ai::get_attachment(&pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let conversation = ai::get_conversation(&pool, attachment.conversation_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if conversation.owner_user_id != Some(user.id) {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok((
        [(header::CONTENT_TYPE, attachment.media_type)],
        Body::from(attachment.data),
    ))
}
