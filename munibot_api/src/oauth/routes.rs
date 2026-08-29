//! Plain axum routes for every provider's oauth2 dance, and logout.
//!
//! These are hand-written handlers rather than dioxus server functions: the
//! browser follows them as ordinary redirects (a provider's consent screen,
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

use crate::{
    auth::server::AuthSession,
    oauth::{discord, github},
};

/// Mounts every provider's `/auth/<provider>/authorize` and
/// `/auth/<provider>/callback`, plus `/auth/logout`.
pub fn router() -> Router {
    Router::new()
        .route("/auth/discord/authorize", get(authorize_discord))
        .route("/auth/discord/callback", get(callback_discord))
        .route("/auth/github/authorize", get(authorize_github))
        .route("/auth/github/callback", get(callback_github))
        .route("/auth/logout", get(logout))
}

/// A provider isn't configured (its client id/secret env vars aren't set) -
/// the same response shape for every provider, so a misconfigured server
/// fails alike regardless of which sign-in button was clicked.
fn not_configured(provider: &str) -> impl IntoResponse {
    error!("{provider} isn't configured; can't sign in with it");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "sign-in isn't configured right now :<",
    )
        .into_response()
}

/// Redirects to discord's consent screen.
async fn authorize_discord() -> impl IntoResponse {
    match (
        std::env::var("MUNIBOT_BASE_URL"),
        std::env::var("DISCORD_APPLICATION_ID"),
    ) {
        (Ok(base_url), Ok(client_id)) => {
            Redirect::to(&discord::authorize_url(&base_url, &client_id)).into_response()
        }
        _ => not_configured("discord").into_response(),
    }
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    error: Option<String>,
}

/// Handles discord's redirect back after the user accepts or declines.
async fn callback_discord(
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

/// Redirects to GitHub's consent screen.
async fn authorize_github() -> impl IntoResponse {
    match (
        std::env::var("MUNIBOT_BASE_URL"),
        std::env::var("GITHUB_OAUTH_CLIENT_ID"),
    ) {
        (Ok(base_url), Ok(client_id)) => {
            Redirect::to(&github::authorize_url(&base_url, &client_id)).into_response()
        }
        _ => not_configured("github sign-in").into_response(),
    }
}

/// Handles GitHub's redirect back after the user accepts or declines.
async fn callback_github(
    auth: AuthSession,
    Query(params): Query<CallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    let Some(code) = params.code else {
        warn!(error = ?params.error, "github oauth callback without a code");
        return Redirect::to("/");
    };

    match sign_in_with_github(&pool, &code).await {
        Ok(user_id) => {
            auth.login_user(user_id.to_string());
            Redirect::to("/dashboard")
        }
        Err(e) => {
            error!(error = %e, "github sign-in failed");
            Redirect::to("/")
        }
    }
}

/// Exchanges the code, fetches the GitHub identity, and finds or creates the
/// corresponding munibot user. Returns the signed-in user's id.
///
/// `provider_user_id` is GitHub's numeric account id (stable across a
/// username change), not `login` - the same reasoning `GitHubUser`'s own
/// doc comment documents. No refresh token: GitHub OAuth App tokens don't
/// expire, unlike discord's.
async fn sign_in_with_github(pool: &DbPool, code: &str) -> anyhow::Result<i64> {
    let base_url = std::env::var("MUNIBOT_BASE_URL")?;
    let client_id = std::env::var("GITHUB_OAUTH_CLIENT_ID")?;
    let client_secret = std::env::var("GITHUB_OAUTH_CLIENT_SECRET")?;

    let token = github::exchange_code(code, &base_url, &client_id, &client_secret).await?;
    let github_user = github::get_current_user(&token.access_token).await?;

    let user = operations::get_or_create_user_from_linked_account(
        pool,
        "github",
        &github_user.id.to_string(),
        &github_user.login,
        github_user.display_name(),
        github_user.avatar_url.as_deref(),
        &token.access_token,
        None,
        None,
    )
    .await?;

    Ok(user.id)
}

/// Logs the current session out and returns home.
async fn logout(auth: AuthSession) -> Redirect {
    auth.logout_user();
    Redirect::to("/")
}
