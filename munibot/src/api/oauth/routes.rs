//! Plain axum routes for the discord oauth2 dance and logout.
//!
//! These are hand-written handlers rather than dioxus server functions: the
//! browser follows them as ordinary redirects (discord's consent screen,
//! then back to munibot), so there's no client-side rpc call to type.

use axum::{
    Router,
    extract::{Extension, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::get,
};
use munibot_core::db::{DbPool, operations};
use serde::Deserialize;
use tracing::{error, warn};

use crate::api::{auth::server::AuthSession, oauth::discord};

/// Mounts `/auth/discord/authorize`, `/auth/discord/callback`, and
/// `/auth/logout`.
pub fn router() -> Router {
    Router::new()
        .route("/auth/discord/authorize", get(authorize))
        .route("/auth/discord/callback", get(callback))
        .route("/auth/logout", get(logout))
}

/// Redirects to discord's consent screen.
async fn authorize() -> impl IntoResponse {
    match (
        std::env::var("MUNIBOT_BASE_URL"),
        std::env::var("DISCORD_APPLICATION_ID"),
    ) {
        (Ok(base_url), Ok(client_id)) => {
            Redirect::to(&discord::authorize_url(&base_url, &client_id)).into_response()
        }
        _ => {
            error!(
                "MUNIBOT_BASE_URL and/or DISCORD_APPLICATION_ID aren't set; can't sign in with \
                 discord"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "sign-in isn't configured right now :<",
            )
                .into_response()
        }
    }
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    error: Option<String>,
}

/// Handles discord's redirect back after the user accepts or declines.
async fn callback(
    auth: AuthSession,
    Query(params): Query<CallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    let Some(code) = params.code else {
        warn!(error = ?params.error, "discord oauth callback without a code");
        return Redirect::to("/");
    };

    match sign_in_with_discord(&pool, &code).await {
        Ok(user_id) => {
            auth.login_user(user_id.to_string());
            Redirect::to("/dashboard")
        }
        Err(e) => {
            error!(error = %e, "discord sign-in failed");
            Redirect::to("/")
        }
    }
}

/// Exchanges the code, fetches the discord identity, and finds or creates
/// the corresponding munibot user. Returns the signed-in user's id.
async fn sign_in_with_discord(pool: &DbPool, code: &str) -> anyhow::Result<i64> {
    let base_url = std::env::var("MUNIBOT_BASE_URL")?;
    let client_id = std::env::var("DISCORD_APPLICATION_ID")?;
    let client_secret = std::env::var("DISCORD_CLIENT_SECRET")?;

    let token = discord::exchange_code(code, &base_url, &client_id, &client_secret).await?;
    let discord_user = discord::get_current_user(&token.access_token).await?;

    let token_expires_at =
        chrono::Utc::now().naive_utc() + chrono::Duration::seconds(token.expires_in);

    let user = operations::get_or_create_user_from_linked_account(
        pool,
        "discord",
        &discord_user.id,
        &discord_user.username,
        discord_user.display_name(),
        discord_user.avatar_url().as_deref(),
        &token.access_token,
        Some(&token.refresh_token),
        Some(token_expires_at),
    )
    .await?;

    Ok(user.id)
}

/// Logs the current session out and returns home.
async fn logout(auth: AuthSession) -> Redirect {
    auth.logout_user();
    Redirect::to("/")
}
