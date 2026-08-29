//! GitHub OAuth2 client: authorization-code exchange, and the REST calls
//! made with the resulting user access token.
//!
//! A **separate** OAuth App from `munibot_github`'s own GitHub App
//! (`GITHUB_APP_ID`/`GITHUB_APP_PRIVATE_KEY`, used only to mint
//! installation tokens for the autonomous pipeline): that App authenticates
//! *as itself* against repositories it's installed into, while this
//! authenticates a human signing in, the same distinction
//! `oauth::discord` already draws against `munibot_discord`'s own bot
//! token. `GITHUB_OAUTH_CLIENT_ID`/`GITHUB_OAUTH_CLIENT_SECRET` are its own,
//! separate credentials.

use serde::{Deserialize, Serialize};
use thiserror::Error;

const AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const API_BASE: &str = "https://api.github.com";

/// Scope requested during the authorize step: `read:user` is enough for
/// the profile fields munibot actually reads (id, login, name, avatar) -
/// no repository access is ever requested through this flow, that being
/// entirely the separate GitHub App's concern.
const SCOPE: &str = "read:user";

/// Error talking to GitHub's oauth2 or REST endpoints.
#[derive(Debug, Error)]
pub enum GitHubOAuthError {
    #[error("request to github failed :< {0}")]
    Request(#[from] reqwest::Error),

    #[error("github returned an error: {error} ({error_description:?})")]
    GitHub {
        error: String,
        error_description: Option<String>,
    },
}

/// The redirect URI munibot registers with its GitHub OAuth App for the
/// callback. `base_url` is munibot's own public base url, the same
/// argument `oauth::discord::redirect_uri` takes.
pub fn redirect_uri(base_url: &str) -> String {
    format!("{base_url}/auth/github/callback")
}

/// Builds the URL to redirect a user to for GitHub's consent screen.
///
/// `state` is an opaque, unguessable value the caller generated and
/// stored server-side - see `oauth::discord::authorize_url`'s own doc
/// comment for the full CSRF reasoning, which applies identically here.
pub fn authorize_url(base_url: &str, client_id: &str, state: &str) -> String {
    let mut url = reqwest::Url::parse(AUTHORIZE_URL).expect("static url is valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri(base_url))
        .append_pair("scope", SCOPE)
        .append_pair("state", state);
    url.into()
}

/// A successful authorization-code exchange.
///
/// No refresh token: GitHub's own OAuth Apps (unlike its GitHub Apps) issue
/// non-expiring access tokens, so there is nothing to refresh - unlike
/// `oauth::discord::Token`, which always carries one.
pub struct Token {
    pub access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenResponse {
    Success {
        access_token: String,
    },
    Error {
        error: String,
        error_description: Option<String>,
    },
}

/// Exchanges an authorization code (from the callback's `?code=` query
/// parameter) for an access token.
pub async fn exchange_code(
    code: &str,
    base_url: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<Token, GitHubOAuthError> {
    let response = reqwest::Client::new()
        .post(TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", &redirect_uri(base_url)),
        ])
        .send()
        .await?
        .json::<TokenResponse>()
        .await?;

    match response {
        TokenResponse::Success { access_token } => Ok(Token { access_token }),
        TokenResponse::Error {
            error,
            error_description,
        } => Err(GitHubOAuthError::GitHub {
            error,
            error_description,
        }),
    }
}

/// The subset of GitHub's user object munibot cares about.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubUser {
    /// GitHub's own numeric account id, stable across a username change -
    /// this, not `login`, is what `linked_accounts.provider_user_id` stores.
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

impl GitHubUser {
    /// The name to show for this user: their profile name if set, falling
    /// back to their login.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.login)
    }
}

/// Fetches the identity of the user who owns `access_token`.
///
/// `User-Agent` is required by GitHub's API on every request, unlike
/// Discord's; `munibot/<version>` mirrors the header
/// `crate::tools::exa::ExaClient` already sends elsewhere in this codebase.
pub async fn get_current_user(access_token: &str) -> Result<GitHubUser, GitHubOAuthError> {
    Ok(reqwest::Client::new()
        .get(format!("{API_BASE}/user"))
        .bearer_auth(access_token)
        .header("User-Agent", concat!("munibot/", env!("CARGO_PKG_VERSION")))
        .send()
        .await?
        .json::<GitHubUser>()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authorize_url_carries_client_id_redirect_scope_and_state() {
        let url = authorize_url("https://munibot.example.com", "abc123", "csrf-state-value");
        let parsed = reqwest::Url::parse(&url).expect("should be a valid url");
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().collect();

        assert!(url.starts_with(AUTHORIZE_URL));
        assert_eq!(pairs.get("client_id").map(|v| v.as_ref()), Some("abc123"));
        assert_eq!(pairs.get("scope").map(|v| v.as_ref()), Some(SCOPE));
        assert_eq!(
            pairs.get("state").map(|v| v.as_ref()),
            Some("csrf-state-value")
        );
        assert_eq!(
            pairs.get("redirect_uri").map(|v| v.as_ref()),
            Some(redirect_uri("https://munibot.example.com").as_str())
        );
    }

    #[test]
    fn test_redirect_uri_appends_the_github_callback_path() {
        assert_eq!(
            redirect_uri("https://munibot.example.com"),
            "https://munibot.example.com/auth/github/callback"
        );
    }

    #[test]
    fn test_display_name_falls_back_to_login_when_name_is_unset() {
        let user = GitHubUser {
            id: 1,
            login: "octocat".to_string(),
            name: None,
            avatar_url: None,
        };
        assert_eq!(user.display_name(), "octocat");
    }

    #[test]
    fn test_display_name_prefers_the_profile_name() {
        let user = GitHubUser {
            id: 1,
            login: "octocat".to_string(),
            name: Some("The Octocat".to_string()),
            avatar_url: None,
        };
        assert_eq!(user.display_name(), "The Octocat");
    }
}
