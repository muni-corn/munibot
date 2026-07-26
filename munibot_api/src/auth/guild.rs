use munibot_core::db::{DbPool, models::LinkedAccount, operations};

use crate::{
    auth::server::AuthSession,
    oauth::discord,
    settings::{SettingsError, SettingsResult},
};

/// Verifies that the session's signed-in user administers (owns, or has
/// `MANAGE_GUILD` in) the discord guild `guild_id`, returning their
/// discord-linked account so a caller that also needs the user's own oauth
/// token doesn't have to look it up a second time.
///
/// Every settings server function that reads or writes state scoped to a
/// guild must call this first -- nothing else in munibot verifies guild
/// authority today (`is_administered_by_user` is otherwise used only as a
/// display filter in `get_guilds`). This is a real http round trip to
/// discord per call; there's no local cache of guild membership to check
/// against instead.
pub async fn require_guild_admin(
    auth: &AuthSession,
    pool: &DbPool,
    guild_id: &str,
) -> SettingsResult<LinkedAccount> {
    let user = auth
        .current_user
        .clone()
        .ok_or(SettingsError::NotSignedIn)?;

    // a signed-in munibot user with no linked discord account can't
    // administer any discord guild -- this shouldn't normally happen, since
    // sign-in only ever reaches a user through a discord link today, but
    // it's not this function's job to distinguish that from a genuine
    // permissions problem
    let linked_account = operations::get_linked_account(pool, user.id, "discord")
        .await?
        .ok_or(SettingsError::NotGuildAdmin)?;

    let guilds = discord::get_current_user_guilds(&linked_account.access_token).await?;

    let administers = guilds
        .iter()
        .find(|guild| guild.id == guild_id)
        .is_some_and(discord::DiscordGuild::is_administered_by_user);

    if administers {
        Ok(linked_account)
    } else {
        Err(SettingsError::NotGuildAdmin)
    }
}
