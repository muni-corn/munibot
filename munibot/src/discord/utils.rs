use poise::serenity_prelude::{CacheHttp, GuildId, Http, Message, Result, UserId};

use super::DiscordContext;

pub async fn display_name_from_message(msg: &Message, http: impl CacheHttp) -> String {
    msg.author_nick(&http)
        .await
        .or_else(|| msg.author.global_name.clone())
        .unwrap_or_else(|| msg.author.name.clone())
}

pub async fn display_name_from_command_context(ctx: DiscordContext<'_>) -> String {
    let author = ctx.author();

    if let Some(guild_id) = ctx.guild_id() {
        author
            .nick_in(ctx.http(), guild_id)
            .await
            .or_else(|| author.global_name.clone())
            .unwrap_or_else(|| author.name.clone())
    } else {
        author.name.clone()
    }
}

pub async fn display_name_from_ids(
    http: &Http,
    user_id: UserId,
    guild_id: Option<GuildId>,
) -> Result<String, poise::serenity_prelude::Error> {
    let user = http.get_user(user_id).await?;
    if let Some(guild_id) = guild_id {
        Ok(user
            .nick_in(http, guild_id)
            .await
            .or(user.global_name)
            .unwrap_or(user.name))
    } else {
        Ok(user.global_name.unwrap_or(user.name))
    }
}
