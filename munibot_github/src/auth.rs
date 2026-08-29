//! GitHub App authentication: minting the app's own JWT, exchanging it for
//! per-installation access tokens, and caching those tokens between calls.

use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use jsonwebtoken::EncodingKey;
use octocrab::{
    Octocrab,
    models::{AppId, InstallationId},
};
use secrecy::ExposeSecret;

use crate::error::GitHubError;

/// How long before a cached token's real expiry munibot proactively mints a
/// replacement. Installation tokens live for exactly one hour (see
/// [`TOKEN_LIFETIME`]'s own doc comment) -- refreshing a few minutes early
/// means an in-flight pipeline run never races a request against the
/// token's own expiry.
const REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);

/// GitHub installation access tokens are valid for exactly one hour from
/// mint -- <https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation>.
const TOKEN_LIFETIME: Duration = Duration::from_secs(60 * 60);

/// Mints a fresh installation access token, with no caching of its own --
/// [`InstallationTokenCache`] is what adds caching on top of this.
///
/// A trait rather than a bare async fn so tests can substitute a mock that
/// never makes a real network call to github; [`OctocrabTokenMinter`] is
/// the one production implementation.
#[async_trait]
pub trait TokenMinter: Send + Sync {
    async fn mint(&self, installation_id: InstallationId) -> Result<String, GitHubError>;
}

/// Mints tokens by authenticating as a GitHub App and exchanging its JWT
/// for a per-installation access token, over a real `octocrab` client.
pub struct OctocrabTokenMinter {
    app_client: Octocrab,
}

impl OctocrabTokenMinter {
    /// Builds an app-authenticated client from `app_id` and `private_key`
    /// (the PEM-encoded RSA key that is `GITHUB_APP_PRIVATE_KEY`'s own
    /// contents).
    ///
    /// `octocrab` signs and refreshes the app-level JWT itself on every
    /// request that needs one; this crate's own responsibility starts one
    /// level down, at the per-installation access token that JWT is
    /// exchanged for (see [`InstallationTokenCache`]).
    pub fn new(app_id: AppId, private_key: &str) -> Result<Self, GitHubError> {
        let key = EncodingKey::from_rsa_pem(private_key.as_bytes())
            .map_err(|error| GitHubError::Config(format!("invalid app private key: {error}")))?;
        let app_client = Octocrab::builder()
            .app(app_id, key)
            .build()
            .map_err(|error| GitHubError::Config(format!("couldn't build app client: {error}")))?;
        Ok(Self { app_client })
    }

    /// Builds an `Octocrab` client scoped to one installation, for making
    /// ordinary REST calls (fetching an issue, posting a comment, opening a
    /// pull request) -- see [`crate::forge::GitHubForge`].
    ///
    /// Cheap and does no network I/O of its own: `octocrab` mints and
    /// caches this client's own token lazily, the first time it actually
    /// makes a request.
    pub fn installation_client(
        &self,
        installation_id: InstallationId,
    ) -> Result<Octocrab, GitHubError> {
        self.app_client
            .installation(installation_id)
            .map_err(|error| GitHubError::Auth(error.to_string()))
    }
}

#[async_trait]
impl TokenMinter for OctocrabTokenMinter {
    async fn mint(&self, installation_id: InstallationId) -> Result<String, GitHubError> {
        let (_, token) = self
            .app_client
            .installation_and_token(installation_id)
            .await
            .map_err(|error| GitHubError::Auth(error.to_string()))?;
        Ok(token.expose_secret().to_string())
    }
}

/// Caches per-installation access tokens, refreshing a few minutes before
/// their real one-hour expiry rather than minting a fresh one on every
/// call.
pub struct InstallationTokenCache {
    minter: Box<dyn TokenMinter>,
    tokens: RwLock<HashMap<InstallationId, (String, Instant)>>,
}

impl InstallationTokenCache {
    pub fn new(minter: impl TokenMinter + 'static) -> Self {
        Self {
            minter: Box::new(minter),
            tokens: RwLock::new(HashMap::new()),
        }
    }

    /// Returns a cached token for `installation_id` when it has more than
    /// [`REFRESH_BUFFER`] left before expiring, minting and caching a fresh
    /// one otherwise.
    pub async fn token_for(&self, installation_id: InstallationId) -> Result<String, GitHubError> {
        if let Some(token) = self.cached_if_fresh(installation_id) {
            return Ok(token);
        }

        let token = self.minter.mint(installation_id).await?;
        let expires_at = Instant::now() + TOKEN_LIFETIME;
        self.tokens
            .write()
            .expect("token cache lock poisoned")
            .insert(installation_id, (token.clone(), expires_at));
        Ok(token)
    }

    fn cached_if_fresh(&self, installation_id: InstallationId) -> Option<String> {
        let tokens = self.tokens.read().expect("token cache lock poisoned");
        let (token, expires_at) = tokens.get(&installation_id)?;
        (Instant::now() + REFRESH_BUFFER < *expires_at).then(|| token.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A test-only, throwaway rsa key -- never used against a real github
    /// app, and generated solely to give `EncodingKey::from_rsa_pem`
    /// something valid to parse.
    const TEST_PRIVATE_KEY: &str =
        include_str!("../tests/fixtures/test-only-not-a-real-secret.pem");

    struct MockMinter {
        calls: AtomicUsize,
        token_prefix: &'static str,
    }

    impl MockMinter {
        fn new(token_prefix: &'static str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                token_prefix,
            }
        }
    }

    #[async_trait]
    impl TokenMinter for MockMinter {
        async fn mint(&self, installation_id: InstallationId) -> Result<String, GitHubError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(format!(
                "{}-{}-call{}",
                self.token_prefix, installation_id.0, call
            ))
        }
    }

    #[tokio::test]
    async fn test_token_for_mints_on_first_call() {
        let cache = InstallationTokenCache::new(MockMinter::new("token"));
        let token = cache.token_for(InstallationId(1)).await.unwrap();
        assert_eq!(token, "token-1-call1");
    }

    #[tokio::test]
    async fn test_token_for_reuses_a_fresh_cached_token() {
        let cache = InstallationTokenCache::new(MockMinter::new("token"));
        let first = cache.token_for(InstallationId(1)).await.unwrap();
        let second = cache.token_for(InstallationId(1)).await.unwrap();

        assert_eq!(first, second, "a fresh token should never be re-minted");
    }

    #[tokio::test]
    async fn test_token_for_mints_independently_per_installation() {
        let cache = InstallationTokenCache::new(MockMinter::new("token"));
        let one = cache.token_for(InstallationId(1)).await.unwrap();
        let two = cache.token_for(InstallationId(2)).await.unwrap();

        assert_ne!(one, two, "each installation should get its own token");
    }

    #[tokio::test]
    async fn test_token_for_refreshes_a_token_within_the_refresh_buffer_of_expiring() {
        let cache = InstallationTokenCache::new(MockMinter::new("token"));
        let stale = cache.token_for(InstallationId(1)).await.unwrap();

        // simulate the cached token being within the refresh buffer of its
        // real expiry, without waiting a wall-clock hour for it to happen
        cache.tokens.write().unwrap().insert(
            InstallationId(1),
            (stale.clone(), Instant::now() + Duration::from_secs(60)),
        );

        let refreshed = cache.token_for(InstallationId(1)).await.unwrap();
        assert_ne!(
            stale, refreshed,
            "a token within the refresh buffer of expiring should be replaced"
        );
    }

    #[tokio::test]
    async fn test_token_for_does_not_refresh_a_token_well_outside_the_buffer() {
        let cache = InstallationTokenCache::new(MockMinter::new("token"));
        let fresh = cache.token_for(InstallationId(1)).await.unwrap();

        // well past the refresh buffer, nowhere near the real one-hour expiry
        cache.tokens.write().unwrap().insert(
            InstallationId(1),
            (fresh.clone(), Instant::now() + Duration::from_secs(1800)),
        );

        let still_cached = cache.token_for(InstallationId(1)).await.unwrap();
        assert_eq!(fresh, still_cached);
    }

    #[tokio::test]
    async fn test_octocrab_token_minter_accepts_a_valid_rsa_private_key() {
        // building the app client spawns a task on the current runtime, so
        // this needs a tokio context even though it awaits nothing itself
        let minter = OctocrabTokenMinter::new(AppId(123456), TEST_PRIVATE_KEY);
        assert!(minter.is_ok());
    }

    #[tokio::test]
    async fn test_octocrab_token_minter_rejects_a_malformed_private_key() {
        // Octocrab (not Debug) makes expect_err unusable here -- match instead
        match OctocrabTokenMinter::new(AppId(123456), "not a real pem") {
            Ok(_) => panic!("a malformed key should be rejected"),
            Err(error) => assert!(error.to_string().contains("invalid app private key")),
        }
    }
}
