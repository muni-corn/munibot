//! Keeps a linked account's discord access token usable, refreshing it
//! first if it's expired (or close to it).
use chrono::{Duration as ChronoDuration, Utc};
use munibot_core::db::{DbPool, models::LinkedAccount, operations};

use super::credentials::Credentials;
use crate::oauth::discord;

/// How far ahead of a token's real expiry munibot treats it as already
/// expired. Without this margin, a token could pass this check and then
/// expire before the caller's actual discord request lands.
const EXPIRY_SKEW: ChronoDuration = ChronoDuration::seconds(60);

/// Returns `linked_account`'s access token, refreshing and persisting a new
/// one first if the current one is expired (or within `EXPIRY_SKEW` of
/// expiring) and a refresh token is on file.
///
/// Falls back to returning the (possibly stale) token as-is if there's
/// nothing to refresh from, or no expiry on file at all -- the caller's
/// subsequent discord call will fail on its own if the token really is bad,
/// same as it always has.
pub async fn access_token_for(
    pool: &DbPool,
    linked_account: &LinkedAccount,
) -> anyhow::Result<String> {
    let Some(expires_at) = linked_account.token_expires_at else {
        return Ok(linked_account.access_token.clone());
    };

    if Utc::now().naive_utc() + EXPIRY_SKEW < expires_at {
        return Ok(linked_account.access_token.clone());
    }

    let Some(refresh_token) = &linked_account.refresh_token else {
        return Ok(linked_account.access_token.clone());
    };

    let credentials = Credentials::from_env()?;
    let token = discord::refresh_access_token(
        refresh_token,
        &credentials.client_id,
        &credentials.client_secret,
    )
    .await?;

    let token_expires_at = Utc::now().naive_utc() + ChronoDuration::seconds(token.expires_in);

    operations::update_linked_account_tokens(
        pool,
        linked_account.id,
        &token.access_token,
        Some(&token.refresh_token),
        Some(token_expires_at),
    )
    .await?;

    Ok(token.access_token)
}
