//! Plain axum routes for every provider's oauth2 dance, and logout.
//!
//! These are hand-written handlers rather than dioxus server functions: the
//! browser follows them as ordinary redirects (a provider's consent screen,
//! then back to munibot), so there's no client-side rpc call to type.

use axum::{
    Router,
    extract::{Extension, Form, Query},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use munibot_core::db::{DbPool, operations};
use serde::Deserialize;
use tracing::{error, warn};

use crate::{
    auth::server::AuthSession,
    oauth::{discord, email, github},
};

/// Mounts every provider's `/auth/<provider>/authorize` and
/// `/auth/<provider>/callback` (email's own shape is a form POST plus a
/// callback, rather than an authorize redirect - see its own handlers),
/// plus `/auth/logout`.
pub fn router() -> Router {
    Router::new()
        .route("/auth/discord/authorize", get(authorize_discord))
        .route("/auth/discord/callback", get(callback_discord))
        .route("/auth/github/authorize", get(authorize_github))
        .route("/auth/github/callback", get(callback_github))
        .route("/auth/email/request", post(request_email))
        .route("/auth/email/callback", get(callback_email))
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

#[derive(Deserialize)]
struct EmailRequestForm {
    email: String,
}

/// Handles the email sign-in form's POST: sends a magic link and always
/// shows the same "check your email" response - never revealing whether
/// mail delivery is even configured, the address already has an account,
/// or the request otherwise failed, the same enumeration-avoidance
/// reasoning `email::request_signin`'s own doc comment documents. A real
/// failure is still logged server-side, just never surfaced to the caller.
async fn request_email(
    Extension(pool): Extension<DbPool>,
    Form(form): Form<EmailRequestForm>,
) -> impl IntoResponse {
    let checked_your_email = (
        StatusCode::OK,
        Html(
            "check your email for a sign-in link! it'll work for the next 15 minutes. (if nothing \
             arrives, mail might not be set up on this server yet.)",
        ),
    );

    let Ok(base_url) = std::env::var("MUNIBOT_BASE_URL") else {
        warn!("MUNIBOT_BASE_URL isn't set; can't build an email sign-in link");
        return checked_your_email.into_response();
    };

    let mailer = match crate::mailer::Mailer::from_env() {
        Some(Ok(mailer)) => mailer,
        Some(Err(error)) => {
            error!(%error, "couldn't set up the smtp mailer");
            return checked_your_email.into_response();
        }
        None => {
            warn!("SMTP_HOST isn't set; email sign-in isn't configured");
            return checked_your_email.into_response();
        }
    };

    if let Err(error) = email::request_signin(&pool, &mailer, &base_url, &form.email).await {
        warn!(%error, email = %form.email, "email sign-in request failed");
    }

    checked_your_email.into_response()
}

/// Handles a magic link's callback.
async fn callback_email(
    auth: AuthSession,
    Query(params): Query<EmailCallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    let Some(token) = params.token else {
        warn!("email sign-in callback without a token");
        return Redirect::to("/");
    };

    match email::verify_signin(&pool, &token).await {
        Ok(Some(user_id)) => {
            auth.login_user(user_id.to_string());
            Redirect::to("/dashboard")
        }
        Ok(None) => {
            warn!("an email sign-in link was invalid, already used, or expired");
            Redirect::to("/")
        }
        Err(error) => {
            error!(%error, "email sign-in failed");
            Redirect::to("/")
        }
    }
}

#[derive(Deserialize)]
struct EmailCallbackParams {
    token: Option<String>,
}

/// Logs the current session out and returns home.
async fn logout(auth: AuthSession) -> Redirect {
    auth.logout_user();
    Redirect::to("/")
}
