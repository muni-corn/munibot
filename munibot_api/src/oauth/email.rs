//! Passwordless "magic link" email sign-in.
//!
//! Distinct from `oauth::discord`/`oauth::github`: there is no
//! third-party consent screen here - munibot mails a single-use,
//! time-limited token directly, generated and verified entirely by this
//! module. Still stored the same way every other provider is:
//! `linked_accounts` with `provider = "email"` and the address itself as
//! `provider_user_id`, via the same provider-generic
//! `get_or_create_user_from_linked_account` every other provider already
//! reuses - no separate identity model, matching the milestone 6 plan's
//! own framing.

use chrono::Duration;
use munibot_core::db::{DbPool, operations};
use rand::RngExt;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// How long a magic link stays valid before [`verify_signin`] refuses it.
const TOKEN_TTL: Duration = Duration::minutes(15);

/// 256 bits of randomness - plenty to make guessing a live token
/// infeasible, the same order of magnitude a session id or an API key
/// would use.
const TOKEN_BYTES: usize = 32;

/// Errors requesting or completing an email sign-in.
#[derive(Debug, Error)]
pub enum EmailSigninError {
    #[error("that doesn't look like an email address")]
    InvalidAddress,
    #[error(transparent)]
    Mailer(#[from] crate::mailer::MailerError),
    #[error(transparent)]
    Database(#[from] diesel::result::Error),
}

/// Generates a fresh token, stores only its hash, and emails the resulting
/// callback link.
///
/// `base_url` is munibot's own public base url, the same argument every
/// other provider's own `redirect_uri` takes. Deliberately does not
/// distinguish "no such account yet" from "here's your link" in its
/// return value - the same enumeration-avoidance reasoning
/// `owned_conversation` documents elsewhere, since either way the caller
/// (the route handler) shows an identical "check your email" response.
pub async fn request_signin(
    pool: &DbPool,
    mailer: &crate::mailer::Mailer,
    base_url: &str,
    email: &str,
) -> Result<(), EmailSigninError> {
    // deliberately not a real RFC 5321 validator - just enough to reject an
    // obviously-empty or malformed field before spending a database write
    // and an outbound smtp connection on it; lettre's own address parser in
    // Mailer::send_signin_link is the actual, authoritative check
    if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
        return Err(EmailSigninError::InvalidAddress);
    }

    let token = generate_token();
    let token_hash = hash_token(&token);
    let expires_at = (chrono::Utc::now() + TOKEN_TTL).naive_utc();

    operations::upsert_email_signin_token(pool, email, &token_hash, expires_at).await?;

    let link = format!("{base_url}/auth/email/callback?token={token}");
    mailer.send_signin_link(email, &link).await?;

    Ok(())
}

/// Consumes a token from a callback's `?token=` query parameter, returning
/// the signed-in user's id when it was valid and unexpired, or `None`
/// when it was missing, already used, or expired - the caller shows the
/// same "that link didn't work" response either way, never leaking which.
pub async fn verify_signin(pool: &DbPool, token: &str) -> Result<Option<i64>, EmailSigninError> {
    let token_hash = hash_token(token);
    let Some(email) = operations::consume_email_signin_token(pool, &token_hash).await? else {
        return Ok(None);
    };

    let user = operations::get_or_create_user_from_linked_account(
        pool, "email", &email, &email, &email, None,
        // an email sign-in has no oauth access token at all --
        // linked_accounts.access_token is NOT NULL, so an empty string
        // stands in for "not applicable" here, never a real credential
        "", None, None,
    )
    .await?;

    Ok(Some(user.id))
}

/// A fresh random token, as lowercase hex.
fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill(&mut bytes);
    to_hex(&bytes)
}

/// `token`'s SHA-256 digest as lowercase hex - the one-way transform that
/// lets `email_signin_tokens` verify a token without ever storing the
/// value that was actually mailed out.
fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    to_hex(&hasher.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_is_64_hex_characters() {
        let token = generate_token();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_token_is_not_deterministic() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn test_hash_token_is_deterministic() {
        assert_eq!(hash_token("some-token"), hash_token("some-token"));
    }

    #[test]
    fn test_hash_token_differs_for_different_tokens() {
        assert_ne!(hash_token("token-a"), hash_token("token-b"));
    }

    #[test]
    fn test_hash_token_never_reveals_the_original_token() {
        let token = generate_token();
        assert_ne!(hash_token(&token), token);
    }
}
