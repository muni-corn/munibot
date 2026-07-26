use dioxus::prelude::*;

/// Returns munibot's discord server invite link, if one is configured, for
/// an "invite munibot" call to action on a guild munibot isn't in yet.
///
/// The extractor is referenced by full path inside the attribute, same as
/// every other server function here: `munibot_core` is an optional,
/// server-only dependency, so a top-level `use` of it would fail to
/// resolve when compiling the wasm client.
#[server(discord_config: axum::extract::Extension<munibot_core::config::DiscordConfig>)]
pub async fn get_discord_invite_link() -> Result<Option<String>, ServerFnError> {
    Ok(discord_config.invite_link.clone())
}
