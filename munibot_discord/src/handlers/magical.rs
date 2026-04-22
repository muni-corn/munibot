use munibot_core::magical::get_magic_message;

use crate::{
    DiscordCommand, DiscordContext, commands::DiscordCommandProvider, error::MuniBotError,
    utils::display_name_from_command_context,
};

/// Discord magical handler. Uses munibot_core::magical for the calculation,
/// implementing only the DiscordCommandProvider side.
pub struct MagicalHandler;

/// Check your magicalness today.
#[poise::command(prefix_command, slash_command)]
async fn magical(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    let nick = display_name_from_command_context(ctx).await;

    ctx.say(get_magic_message(&ctx.author().id.to_string(), &nick))
        .await?;

    Ok(())
}

impl DiscordCommandProvider for MagicalHandler {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![magical()]
    }
}
