use poise::serenity_prelude::{
    ChannelId, Context, FullEvent, GuildId, MessageBuilder, Result, UserId, async_trait,
};

use super::{
    DiscordFrameworkContext,
    handler::{DiscordEventHandler, DiscordHandlerError},
    utils::display_name_from_ids,
};

pub struct VoiceChannelGreeter;

#[async_trait]
impl DiscordEventHandler for VoiceChannelGreeter {
    fn name(&self) -> &'static str {
        "vc greeter"
    }

    async fn handle_discord_event(
        &mut self,
        context: &Context,
        _framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        if let FullEvent::VoiceStateUpdate { old, new } = event {
            if let Some(old) = old {
                if old.channel_id != new.channel_id {
                    if let Some(old_channel_id) = old.channel_id {
                        farewell_user(context, old_channel_id, old.user_id, old.guild_id).await?;
                    }
                    if let Some(new_channel_id) = new.channel_id {
                        greet_user(context, new_channel_id, new.user_id, new.guild_id).await?;
                    }
                }
            } else if let Some(channel_id) = new.channel_id {
                greet_user(context, channel_id, new.user_id, new.guild_id).await?;
            }
        }
        Ok(())
    }
}

async fn greet_user(
    ctx: &Context,
    channel_id: ChannelId,
    user_id: UserId,
    guild_id: Option<GuildId>,
) -> Result<(), DiscordHandlerError> {
    let name = display_name_from_ids(&ctx.http, user_id, guild_id)
        .await
        .map_err(|e| DiscordHandlerError {
            handler_name: "vc_greeter",
            message: format!("couldn't get display name: {e}"),
        })?;

    let msg = MessageBuilder::new().push("hi, ").push_safe(name).build();

    channel_id
        .say(&ctx.http, msg)
        .await
        .map_err(|e| DiscordHandlerError {
            handler_name: "vc_greeter",
            message: format!("couldn't send vc greeting: {e}"),
        })?;

    Ok(())
}

async fn farewell_user(
    ctx: &Context,
    channel_id: ChannelId,
    user_id: UserId,
    guild_id: Option<GuildId>,
) -> Result<(), DiscordHandlerError> {
    let name = display_name_from_ids(&ctx.http, user_id, guild_id)
        .await
        .map_err(|e| DiscordHandlerError {
            handler_name: "vc_greeter",
            message: format!("couldn't get display name: {e}"),
        })?;

    channel_id
        .say(&ctx.http, format!("bye, {name}!"))
        .await
        .map_err(|e| DiscordHandlerError {
            handler_name: "vc_greeter",
            message: format!("couldn't send vc farewell: {e}"),
        })?;

    Ok(())
}
