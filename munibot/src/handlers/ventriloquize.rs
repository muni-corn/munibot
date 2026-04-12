use std::time::Duration;

use log::error;
use poise::{Command, CreateReply, serenity_prelude::CreateMessage};
use tokio::time::sleep;

use crate::{
    MuniBotError,
    discord::{DiscordContext, commands::DiscordCommandProvider, state::DiscordState},
};

pub struct VentriloquizeProvider;

#[poise::command(slash_command, hide_in_help, check = "is_ventriloquist")]
async fn ventriloquize<'a, 'b: 'a>(
    ctx: DiscordContext<'b>,
    message: String,
) -> Result<(), MuniBotError> {
    let channel_id = ctx.channel_id();
    let http = ctx.serenity_context().http.to_owned();

    // notification the command invoker
    let reply = CreateReply::default()
        .ephemeral(true)
        .content("beep boop...");
    ctx.send(reply).await?;

    tokio::spawn(async move {
        // start typing to look like munibot is actually typing
        let typing = channel_id.start_typing(&http);

        // wait a minute to simulate typing
        sleep(Duration::from_millis(message.len() as u64 * 25)).await;
        typing.stop();

        // send the message
        let message = CreateMessage::default().content(message);
        if let Err(e) = channel_id.send_message(&http, message).await {
            error!("couldn't send ventriloquization: {e}");
        }
    });

    Ok(())
}

impl DiscordCommandProvider for VentriloquizeProvider {
    fn commands(&self) -> Vec<Command<DiscordState, MuniBotError>> {
        vec![ventriloquize()]
    }
}

async fn is_ventriloquist(ctx: DiscordContext<'_>) -> Result<bool, MuniBotError> {
    Ok(ctx
        .data()
        .config
        .ventriloquists
        .iter()
        .any(|id| *id == ctx.author().id))
}
