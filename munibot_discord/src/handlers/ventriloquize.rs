use std::time::Duration;

use poise::{Command, CreateReply, serenity_prelude::CreateMessage};
use tokio::time::sleep;
use tracing::{Instrument, error, info_span, instrument};

use crate::{
    DiscordContext, commands::DiscordCommandProvider, error::MuniBotError, state::DiscordState,
};

pub struct VentriloquizeProvider;

#[poise::command(slash_command, hide_in_help, check = "is_ventriloquist")]
#[instrument(skip_all, fields(
    user = %ctx.author().name,
    guild = ?ctx.guild_id(),
    channel = %ctx.channel_id()
))]
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

    // propagate span context into the spawned task
    let send_span = info_span!("ventriloquize_send", channel = %channel_id);
    tokio::spawn(
        async move {
            // start typing to look like munibot is actually typing
            let typing = channel_id.start_typing(&http);

            // wait to simulate typing
            sleep(Duration::from_millis(message.len() as u64 * 25)).await;
            typing.stop();

            // send the message
            let message = CreateMessage::default().content(message);
            if let Err(e) = channel_id.send_message(&http, message).await {
                error!(error = %e, "couldn't send ventriloquization");
            }
        }
        .instrument(send_span),
    );

    Ok(())
}

impl DiscordCommandProvider for VentriloquizeProvider {
    fn commands(&self) -> Vec<Command<DiscordState, MuniBotError>> {
        vec![ventriloquize()]
    }
}

async fn is_ventriloquist(ctx: DiscordContext<'_>) -> Result<bool, MuniBotError> {
    let author_id = ctx.author().id.get();
    Ok(ctx.data().config.ventriloquists.contains(&author_id))
}
