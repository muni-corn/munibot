// oauth client code is entirely server-side: it exchanges secrets and
// bearer tokens that must never reach the wasm client.

pub mod discord;
pub mod email;
pub mod github;
pub mod routes;

/// What completing a provider's sign-in flow actually did, once the
/// provider's own identity is known - shared by every provider's callback
/// handler in [`routes`], since all three (discord, github, email) branch
/// on the exact same thing: was someone already signed in when the flow
/// completed?
///
/// A signed-out visitor always gets [`Self::SignedIn`] (creating a user the
/// first time, matching an existing one on a repeat sign-in); someone
/// already signed in gets [`Self::Linked`] or [`Self::AlreadyLinkedElsewhere`]
/// instead - see `munibot_core::db::operations::link_account_to_user`'s own
/// doc comment for why the latter refuses rather than silently reassigning
/// the account.
pub enum LinkOrSignIn {
    SignedIn(i64),
    Linked,
    AlreadyLinkedElsewhere,
}

impl LinkOrSignIn {
    /// Dispatches to
    /// [`munibot_core::db::operations::get_or_create_user_from_linked_account`]
    /// when `existing_user_id` is `None`, or
    /// [`munibot_core::db::operations::link_account_to_user`] otherwise -
    /// the one branch every provider's own `sign_in_with_*`/`verify_*`
    /// function needs, extracted so none of them duplicate it.
    #[allow(clippy::too_many_arguments)]
    async fn resolve(
        pool: &munibot_core::db::DbPool,
        existing_user_id: Option<i64>,
        provider: &str,
        provider_user_id: &str,
        username: &str,
        display_name: &str,
        avatar_url: Option<&str>,
        access_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<chrono::NaiveDateTime>,
    ) -> anyhow::Result<Self> {
        use munibot_core::db::operations;

        match existing_user_id {
            Some(user_id) => Ok(
                match operations::link_account_to_user(
                    pool,
                    user_id,
                    provider,
                    provider_user_id,
                    username,
                    access_token,
                    refresh_token,
                    token_expires_at,
                )
                .await?
                {
                    operations::LinkAccountOutcome::Linked => Self::Linked,
                    operations::LinkAccountOutcome::AlreadyLinkedElsewhere => {
                        Self::AlreadyLinkedElsewhere
                    }
                },
            ),
            None => {
                let user = operations::get_or_create_user_from_linked_account(
                    pool,
                    provider,
                    provider_user_id,
                    username,
                    display_name,
                    avatar_url,
                    access_token,
                    refresh_token,
                    token_expires_at,
                )
                .await?;
                Ok(Self::SignedIn(user.id))
            }
        }
    }
}
