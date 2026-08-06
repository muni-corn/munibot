// permissions.rs: grants configured operators their permission on startup.

use munibot_core::{
    Config, OperatorConfig, Permission,
    db::{DbPool, operations},
};
use tracing::warn;

/// Resolves every configured `[[operators]]` entry to a user and grants them
/// `Permission::Operator`, logging and skipping (rather than failing
/// startup) any entry that doesn't resolve to a real user - most commonly a
/// linked-account entry for someone who hasn't signed in yet.
///
/// Grant-only: removing an entry from config does not revoke a permission
/// already granted. An operator who no longer belongs must be revoked by
/// hand for now - see `docs/notes` for why this was deliberately deferred
/// rather than added speculatively.
pub async fn sync_operators(config: &Config, pool: &DbPool) {
    for entry in &config.operators {
        let user = match entry {
            OperatorConfig::LinkedAccount {
                provider,
                provider_user_id,
            } => operations::find_user_by_linked_account(pool, provider, provider_user_id).await,
            OperatorConfig::MunibotUser { munibot_user_id } => {
                operations::get_user(pool, *munibot_user_id).await
            }
        };

        match user {
            Ok(Some(user)) => {
                if let Err(error) =
                    operations::grant_permission(pool, user.id, &Permission::Operator.to_string())
                        .await
                {
                    warn!(%error, user_id = user.id, "couldn't grant the operator permission");
                }
            }
            Ok(None) => {
                warn!(
                    ?entry,
                    "a configured operator doesn't match any known user yet"
                );
            }
            Err(error) => {
                warn!(%error, ?entry, "couldn't resolve a configured operator");
            }
        }
    }
}
