use poise::{
    CreateReply,
    serenity_prelude::{ChannelId, Mentionable, MessageBuilder},
};

use super::{
    DiscordCommand, DiscordCommandProvider, DiscordContext, autodelete::AutoDeleteHandler,
};
use crate::{
    MuniBotError, db::DbItem, discord::autodelete::AutoDeleteMode,
    handlers::logging::LoggingChannel,
};

pub struct AdminCommandProvider;

impl DiscordCommandProvider for AdminCommandProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![admin()]
    }
}

#[poise::command(
    slash_command,
    hide_in_help,
    required_permissions = "MANAGE_GUILD",
    subcommand_required,
    subcommands("set_log_channel", "stop_logging", "set_autodelete", "stop_autodelete"),
    ephemeral
)]
async fn admin(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    let mut msg = String::from(
        "hi :3 this command has subcommands for managing my server administration tools.",
    );
    if ctx.guild_id().is_some() {
        msg.push_str(" perhaps you meant to use one of them?");
    } else {
        msg.push_str(" you can only use it in a server, silly.");
    }
    ctx.say(msg).await?;
    Ok(())
}

/// set the channel to which log server events will be sent.
#[poise::command(
    rename = "set-log-channel",
    slash_command,
    hide_in_help,
    guild_only,
    required_permissions = "MANAGE_GUILD",
    ephemeral
)]
async fn set_log_channel(
    ctx: DiscordContext<'_>,

    #[description = "the channel to log messages to. if omitted, use the current channel instead."]
    channel: Option<ChannelId>,
) -> Result<(), MuniBotError> {
    let db = &ctx.data().access().db();

    let reply_content = if let Some(guild_id) = ctx.guild_id() {
        let channel_id = channel.unwrap_or_else(|| ctx.channel_id());
        let lc = LoggingChannel::new(guild_id, channel_id);
        lc.upsert_in_db(db, lc.clone()).await?;

        format!(
            "done! log messages will be sent to {}.",
            channel
                .map(|id| id.mention().to_string())
                .unwrap_or("this channel".to_string())
        )
    } else {
        "this command can only be used in a server, silly.".to_string()
    };

    let reply = CreateReply::default()
        .ephemeral(true)
        .content(reply_content);
    ctx.send(reply).await?;
    Ok(())
}

/// stop logging messages in the server. re-enable with `set-log-channel`.
#[poise::command(
    rename = "stop-logging",
    slash_command,
    hide_in_help,
    required_permissions = "MANAGE_GUILD",
    guild_only,
    ephemeral
)]
async fn stop_logging(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    let db = &ctx.data().access().db();

    let reply_content = if let Some(guild_id) = ctx.guild_id() {
        if let Some(logging_entry) = LoggingChannel::get_from_db(db, guild_id).await? {
            logging_entry.delete_from_db(db).await?;
            "done! logging has been disabled for this server."
        } else {
            "no logging channel is set for this server! nothing was done."
        }
    } else {
        "this command can only be used in a server, silly."
    };

    let reply = CreateReply::default()
        .ephemeral(true)
        .content(reply_content);
    ctx.send(reply).await?;
    Ok(())
}

/// setup auto-deleting messages for this channel.
#[poise::command(
    rename = "set-autodelete",
    slash_command,
    hide_in_help,
    required_permissions = "MANAGE_GUILD",
    guild_only,
    ephemeral
)]
async fn set_autodelete(
    ctx: DiscordContext<'_>,

    #[description = "how long messages should survive before deletion, e.g. '1h', '8 hours', '1 week'"]
    duration: String,
    #[description = "whether to always clean any message that is old or only clean messages after the channel is silent"]
    #[rename = "clean_mode"]
    specified_clean_mode: Option<AutoDeleteMode>,
) -> Result<(), MuniBotError> {
    let clean_mode = specified_clean_mode.unwrap_or_default();
    let reply_content = if let Some(guild_id) = ctx.guild_id() {
        let mut msg = MessageBuilder::new();

        let parsed_duration = humantime::parse_duration(&duration)?;
        if parsed_duration >= AutoDeleteHandler::MINIMUM_TIMER_DURATION {
            ctx.framework()
                .user_data
                .autodeletion()
                .lock()
                .await
                .set_autodelete(guild_id, ctx.channel_id(), parsed_duration, clean_mode)
                .await?;

            let formatted_duration = humantime::format_duration(parsed_duration).to_string();

            match clean_mode {
                AutoDeleteMode::Always => {
                    msg.push("okay! this channel will delete messages older than ")
                        .push_bold(&formatted_duration)
                        .push('.');
                }
                AutoDeleteMode::AfterSilence => {
                    msg.push("okay! this channel will delete messages after ")
                        .push_bold(&formatted_duration)
                        .push(" of no activity.");
                }
            }

            if specified_clean_mode.is_none() {
                msg.push(" if you want ignore channel activity and delete ")
                    .push_italic("any")
                    .push(" message that is older than ")
                    .push_bold(formatted_duration)
                    .push(", you can run this command again and set `mode` to \"always\".");
            }
        } else {
            msg.push("i can't delete messages that quickly. give me a duration that's at least ")
                .push_bold(
                    humantime::format_duration(AutoDeleteHandler::MINIMUM_TIMER_DURATION)
                        .to_string(),
                )
                .push('.');
        }
        msg.build()
    } else {
        "you can only use this command in a server, silly.".to_string()
    };

    let reply = CreateReply::default()
        .ephemeral(true)
        .content(reply_content);
    ctx.send(reply).await?;
    Ok(())
}

/// stops autodeletion for this channel.
#[poise::command(
    rename = "stop-autodelete",
    slash_command,
    hide_in_help,
    required_permissions = "MANAGE_GUILD",
    guild_only,
    ephemeral
)]
async fn stop_autodelete(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    let reply_content = if let Some(guild_id) = ctx.guild_id() {
        let did_exist = ctx
            .framework()
            .user_data
            .autodeletion()
            .lock()
            .await
            .clear_autodelete(guild_id, ctx.channel_id())
            .await?;

        if did_exist {
            "done! less cleanup for me to do~"
        } else {
            "i don't have an autodelete timer for this channel :3"
        }
    } else {
        "you can only use this command in a server, silly."
    };

    let reply = CreateReply::default()
        .ephemeral(true)
        .content(reply_content);
    ctx.send(reply).await?;
    Ok(())
}
