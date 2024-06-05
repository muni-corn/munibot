use poise::serenity_prelude::{CacheHttp, Message};

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
