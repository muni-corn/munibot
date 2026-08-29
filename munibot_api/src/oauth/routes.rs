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
use munibot_core::db::DbPool;
use rand::RngExt;
use serde::Deserialize;
use tracing::{error, warn};

use crate::{
    auth::server::AuthSession,
    oauth::{LinkOrSignIn, discord, email, github},
};

/// The session key an authorize-step CSRF state token is stashed under,
/// until the matching callback consumes it - see [`generate_csrf_state`]
/// and [`verify_csrf_state`].
const CSRF_STATE_SESSION_KEY: &str = "oauth_csrf_state";

/// Generates a fresh CSRF state token, stashes it in the session (which
/// already exists, and is already tracked via a cookie, before anyone
/// signs in - `axum_session` issues one to every visitor regardless), and
/// returns it for embedding in a provider's authorize URL.
///
/// `docs/gui.md:132` names the gap this closes: without a `state` round
/// trip, nothing stops a forged callback (an attacker's own authorization
/// code, delivered to a victim's browser) from being accepted as if the
/// victim had actually completed the flow themselves.
fn generate_csrf_state(auth: &AuthSession) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let state = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    auth.session.set(CSRF_STATE_SESSION_KEY, &state);
    state
}

/// Verifies a callback's `state` query parameter against what
/// [`generate_csrf_state`] stashed for this same session, removing it
/// from the session either way - single-use, so a state value can never
/// be replayed against a second callback even if the first one somehow
/// leaked.
fn verify_csrf_state(auth: &AuthSession, provided: Option<&str>) -> bool {
    let expected: Option<String> = auth.session.get(CSRF_STATE_SESSION_KEY);
    auth.session.remove(CSRF_STATE_SESSION_KEY);

    match (expected, provided) {
        (Some(expected), Some(provided)) => expected == provided,
        _ => false,
    }
}

/// Turns a [`LinkOrSignIn`] into the redirect a callback handler shows,
/// logging the session in for a fresh sign-in but leaving an
/// already-signed-in session untouched for a link (there is nothing to
/// change - the same person is still signed in as themself either way).
fn redirect_for(auth: &AuthSession, outcome: LinkOrSignIn, provider: &str) -> Redirect {
    match outcome {
        LinkOrSignIn::SignedIn(user_id) => {
            auth.login_user(user_id.to_string());
            Redirect::to("/dashboard")
        }
        LinkOrSignIn::Linked => Redirect::to("/account"),
        LinkOrSignIn::AlreadyLinkedElsewhere => {
            warn!("attempted to link a {provider} account already linked to a different user");
            Redirect::to("/account?error=already_linked")
        }
    }
}

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
async fn authorize_discord(auth: AuthSession) -> impl IntoResponse {
    match (
        std::env::var("MUNIBOT_BASE_URL"),
        std::env::var("DISCORD_APPLICATION_ID"),
    ) {
        (Ok(base_url), Ok(client_id)) => {
            let state = generate_csrf_state(&auth);
            Redirect::to(&discord::authorize_url(&base_url, &client_id, &state)).into_response()
        }
        _ => not_configured("discord").into_response(),
    }
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    error: Option<String>,
    state: Option<String>,
}

/// Handles discord's redirect back after the user accepts or declines.
///
/// Signed in already (linking another provider to the current account) or
/// not (a normal sign-in) is decided from the session itself, at the top
/// of this handler - see [`LinkOrSignIn`]'s own doc comment.
async fn callback_discord(
    auth: AuthSession,
    Query(params): Query<CallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    if !verify_csrf_state(&auth, params.state.as_deref()) {
        warn!("discord oauth callback failed csrf state verification");
        return Redirect::to("/");
    }
    let Some(code) = params.code else {
        warn!(error = ?params.error, "discord oauth callback without a code");
        return Redirect::to("/");
    };
    let existing_user_id = auth.current_user.as_ref().map(|user| user.id);

    match complete_discord(&pool, &code, existing_user_id).await {
        Ok(outcome) => redirect_for(&auth, outcome, "discord"),
        Err(e) => {
            error!(error = %e, "discord sign-in failed");
            Redirect::to("/")
        }
    }
}

/// Exchanges the code, fetches the discord identity, and either signs in
/// as (or creates) its matching user, or links it to `existing_user_id`.
async fn complete_discord(
    pool: &DbPool,
    code: &str,
    existing_user_id: Option<i64>,
) -> anyhow::Result<LinkOrSignIn> {
    let base_url = std::env::var("MUNIBOT_BASE_URL")?;
    let client_id = std::env::var("DISCORD_APPLICATION_ID")?;
    let client_secret = std::env::var("DISCORD_CLIENT_SECRET")?;

    let token = discord::exchange_code(code, &base_url, &client_id, &client_secret).await?;
    let discord_user = discord::get_current_user(&token.access_token).await?;

    let token_expires_at =
        chrono::Utc::now().naive_utc() + chrono::Duration::seconds(token.expires_in);

    LinkOrSignIn::resolve(
        pool,
        existing_user_id,
        "discord",
        &discord_user.id,
        &discord_user.username,
        discord_user.display_name(),
        discord_user.avatar_url().as_deref(),
        &token.access_token,
        Some(&token.refresh_token),
        Some(token_expires_at),
    )
    .await
}

/// Redirects to GitHub's consent screen.
async fn authorize_github(auth: AuthSession) -> impl IntoResponse {
    match (
        std::env::var("MUNIBOT_BASE_URL"),
        std::env::var("GITHUB_OAUTH_CLIENT_ID"),
    ) {
        (Ok(base_url), Ok(client_id)) => {
            let state = generate_csrf_state(&auth);
            Redirect::to(&github::authorize_url(&base_url, &client_id, &state)).into_response()
        }
        _ => not_configured("github sign-in").into_response(),
    }
}

/// Handles GitHub's redirect back after the user accepts or declines.
///
/// Signed in already or not is decided from the session itself - see
/// `callback_discord`'s own doc comment for the same reasoning.
async fn callback_github(
    auth: AuthSession,
    Query(params): Query<CallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    if !verify_csrf_state(&auth, params.state.as_deref()) {
        warn!("github oauth callback failed csrf state verification");
        return Redirect::to("/");
    }
    let Some(code) = params.code else {
        warn!(error = ?params.error, "github oauth callback without a code");
        return Redirect::to("/");
    };
    let existing_user_id = auth.current_user.as_ref().map(|user| user.id);

    match complete_github(&pool, &code, existing_user_id).await {
        Ok(outcome) => redirect_for(&auth, outcome, "github"),
        Err(e) => {
            error!(error = %e, "github sign-in failed");
            Redirect::to("/")
        }
    }
}

/// Exchanges the code, fetches the GitHub identity, and either signs in as
/// (or creates) its matching user, or links it to `existing_user_id`.
///
/// `provider_user_id` is GitHub's numeric account id (stable across a
/// username change), not `login` - the same reasoning `GitHubUser`'s own
/// doc comment documents. No refresh token: GitHub OAuth App tokens don't
/// expire, unlike discord's.
async fn complete_github(
    pool: &DbPool,
    code: &str,
    existing_user_id: Option<i64>,
) -> anyhow::Result<LinkOrSignIn> {
    let base_url = std::env::var("MUNIBOT_BASE_URL")?;
    let client_id = std::env::var("GITHUB_OAUTH_CLIENT_ID")?;
    let client_secret = std::env::var("GITHUB_OAUTH_CLIENT_SECRET")?;

    let token = github::exchange_code(code, &base_url, &client_id, &client_secret).await?;
    let github_user = github::get_current_user(&token.access_token).await?;

    LinkOrSignIn::resolve(
        pool,
        existing_user_id,
        "github",
        &github_user.id.to_string(),
        &github_user.login,
        github_user.display_name(),
        github_user.avatar_url.as_deref(),
        &token.access_token,
        None,
        None,
    )
    .await
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
///
/// Signed in already (in the same browser that requested the link) or not
/// is decided from the session itself - see `callback_discord`'s own doc
/// comment for the same reasoning.
async fn callback_email(
    auth: AuthSession,
    Query(params): Query<EmailCallbackParams>,
    Extension(pool): Extension<DbPool>,
) -> Redirect {
    let Some(token) = params.token else {
        warn!("email sign-in callback without a token");
        return Redirect::to("/");
    };
    let existing_user_id = auth.current_user.as_ref().map(|user| user.id);

    match email::verify_signin(&pool, &token, existing_user_id).await {
        Ok(Some(outcome)) => redirect_for(&auth, outcome, "email"),
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
