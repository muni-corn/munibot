use poise::serenity_prelude::{EmbedMessageBuilding, MessageBuilder};

use super::DiscordCommandProvider;
use crate::{DiscordContext, MunibotDiscordError, commands::DiscordCommandError};

mod api;

pub struct FoxCommandProvider;
impl DiscordCommandProvider for FoxCommandProvider {
    fn commands(&self) -> Vec<poise::Command<crate::state::DiscordState, MunibotDiscordError>> {
        vec![fox()]
    }
}

#[poise::command(slash_command, prefix_command)]
pub async fn fox(ctx: DiscordContext<'_>) -> Result<(), MunibotDiscordError> {
    let response = api::get_fox().await.map_err(|e| DiscordCommandError {
        message: e.to_string(),
        command_identifier: "fox".to_string(),
    })?;

    let msg = MessageBuilder::new()
        .push_named_link("yip!", response.image)
        .build();

    ctx.reply(msg).await?;
    Ok(())
}
