use std::future::Future;

use async_trait::async_trait;
use poise::serenity_prelude::{self as serenity, *};
use serde::{Deserialize, Serialize};
use surrealdb::{Connection, Surreal};

use crate::{
    db::DbItem,
    discord::{
        handler::{DiscordEventHandler, DiscordHandlerError},
        DiscordFrameworkContext,
    },
};

pub struct LoggingHandler;

#[async_trait]
impl DiscordEventHandler for LoggingHandler {
    fn name(&self) -> &'static str {
        "logging"
    }

    async fn handle_discord_event(
        &mut self,
        context: &serenity::Context,
        framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        let send = |guild_id: GuildId, embed: CreateEmbed| async move {
            Self::map_result(
                send_message(
                    context,
                    &framework,
                    guild_id,
                    CreateMessage::new().embed(embed),
                )
                .await,
            )
        };

        // fml i don't think there's a better way to do this but here we go :D
        match event {
            FullEvent::AutoModActionExecution { execution } => {
                send(
                    execution.guild_id,
                    useless_embed("automod executed an action"),
                )
                .await
            }

            FullEvent::ChannelCreate { channel } => {
                send(
                    channel.guild_id,
                    simple_embed(
                        "new channel created",
                        &format!("channel {} was created", channel.id.mention()),
                    ),
                )
                .await
            }

            FullEvent::CategoryCreate { category } => {
                let msg = MessageBuilder::new()
                    .push("category ")
                    .push_bold(&category.name)
                    .push(" was created")
                    .build();

                send(
                    category.guild_id,
                    simple_embed("new category created", &msg),
                )
                .await
            }

            FullEvent::CategoryDelete { category } => {
                let msg = MessageBuilder::new()
                    .push("category ")
                    .push_bold(&category.name)
                    .push(" was deleted")
                    .build();

                send(category.guild_id, simple_embed("category deleted", &msg)).await
            }

            FullEvent::ChannelDelete { channel, messages } => {
                let mut msg = MessageBuilder::new();
                msg.push("channel ")
                    .push_bold(&channel.name)
                    .push(" was deleted");

                if let Some(messages) = messages {
                    msg.push("along with its ")
                        .push_bold(format!("{} messages", messages.len()));
                }

                send(
                    channel.guild_id,
                    simple_embed("channel deleted", &msg.build()),
                )
                .await
            }

            FullEvent::GuildBanAddition {
                guild_id,
                banned_user,
            } => {
                let msg = MessageBuilder::new()
                    .push(banned_user.id.mention().to_string())
                    .push_safe(format!(
                        " ({}) was banned",
                        banned_user
                            .global_name
                            .as_ref()
                            .unwrap_or(&banned_user.name),
                    ))
                    .build();

                send(*guild_id, simple_embed("user banned", &msg)).await
            }

            FullEvent::GuildBanRemoval {
                guild_id,
                unbanned_user,
            } => {
                let msg = MessageBuilder::new()
                    .push("on ")
                    .push(unbanned_user.id.mention().to_string())
                    .push_safe(format!(
                        " ({})",
                        unbanned_user
                            .global_name
                            .as_ref()
                            .unwrap_or(&unbanned_user.name),
                    ))
                    .build();

                send(*guild_id, simple_embed("user ban lifted", &msg)).await
            }

            FullEvent::GuildMemberAddition { new_member } => {
                let mut msg = MessageBuilder::new();
                msg.push(new_member.user.id.mention().to_string())
                    .push_safe(format!(
                        " ({}) joined!",
                        new_member
                            .user
                            .global_name
                            .as_ref()
                            .unwrap_or(&new_member.user.name)
                    ));

                let account_created_timestamp = new_member.user.created_at().timestamp();

                let mut embed = embed_with_fields(
                    "new member joined",
                    &msg.build(),
                    vec![(
                        "account created".into(),
                        format!(
                            "<t:{}:F>, <t:{}:R>",
                            account_created_timestamp, account_created_timestamp
                        ),
                        false,
                    )],
                );
                if let Some(timestamp) = new_member.joined_at {
                    embed = embed.timestamp(timestamp);
                }

                send(new_member.guild_id, embed).await
            }

            FullEvent::GuildMemberRemoval { guild_id, user, .. } => {
                let msg = MessageBuilder::new()
                    .push(user.id.mention().to_string())
                    .push_safe(format!(
                        " ({}) has left",
                        user.global_name.as_ref().unwrap_or(&user.name),
                    ))
                    .build();

                send(*guild_id, simple_embed("member left", &msg)).await
            }

            FullEvent::GuildMemberUpdate {
                old_if_available,
                new,
                event,
            } => handle_member_update(send, old_if_available, new, event).await,

            FullEvent::GuildRoleCreate { new } => {
                let msg = MessageBuilder::new()
                    .push(new.mention().to_string())
                    .push(" was created")
                    .build();

                send(new.guild_id, simple_embed("new role created", &msg)).await
            }

            FullEvent::GuildRoleDelete {
                guild_id,
                removed_role_id,
                removed_role_data_if_available,
            } => {
                let mut msg = MessageBuilder::new();
                msg.push("role ");

                if let Some(removed_role_data) = removed_role_data_if_available {
                    msg.push_bold(removed_role_data.name.to_string()).push(" ");
                }

                msg.push("with id ")
                    .push_mono(removed_role_id.to_string())
                    .push(" was deleted");

                send(*guild_id, simple_embed("role deleted", &msg.build())).await
            }
            FullEvent::GuildRoleUpdate {
                old_data_if_available,
                new,
            } => {
                if let Some(old) = old_data_if_available {
                    handle_role_update(send, old, new).await
                } else {
                    send(
                        new.guild_id,
                        simple_embed(
                            "role updated",
                            &format!(
                                "{} was updated, but its old data is not available in the cache",
                                new.mention()
                            ),
                        ),
                    )
                    .await
                }
            }

            FullEvent::GuildUpdate {
                old_data_if_available: _,
                new_data,
            } => send(new_data.id, useless_embed("guild updated")).await,

            FullEvent::InviteCreate { data } => {
                let title = if data.temporary {
                    "temporary invite created"
                } else {
                    "invite created"
                };

                if let Some(guild_id) = data.guild_id {
                    let mut msg = MessageBuilder::new();
                    if let Some(user) = &data.inviter {
                        msg.push(format!("by {} ", user.mention()));
                    }

                    let max_uses = match data.max_uses {
                        0 => "infinite uses".to_string(),
                        1 => "a single use".to_string(),
                        x => format!("{x} maximum uses"),
                    };

                    msg.push("with code ")
                        .push_mono(&data.code)
                        .push(format!(", for channel <#{}>", data.channel_id))
                        .push(", with ")
                        .push_bold(max_uses)
                        .push(", and a lifetime of ")
                        .push_bold(format!("{} seconds", data.max_age));

                    send(guild_id, simple_embed(title, &msg.build())).await
                } else {
                    Ok(())
                }
            }

            FullEvent::InviteDelete { data } => {
                if let Some(guild_id) = data.guild_id {
                    let msg = MessageBuilder::new()
                        .push("with code ")
                        .push_mono(&data.code)
                        .push(format!(" for channel <#{}>", data.channel_id))
                        .build();

                    send(guild_id, simple_embed("invite deleted", &msg)).await
                } else {
                    Ok(())
                }
            }

            FullEvent::MessageDelete {
                channel_id,
                deleted_message_id,
                guild_id,
            } => {
                if let Some(guild_id) = guild_id {
                    let mut msg = MessageBuilder::new();
                    let mut fields = vec![];

                    if let Some(cache) = context.cache() {
                        if let Some(deleted_message) = cache.message(channel_id, deleted_message_id)
                        {
                            msg.push("a message from ")
                                .push(deleted_message.author.mention().to_string())
                                .push(" in ")
                                .push(channel_id.mention().to_string())
                                .push(" was deleted");

                            fields.push((
                                "message content".into(),
                                deleted_message.content_safe(cache),
                                false,
                            ));
                        } else {
                            msg.push("a message in ")
                                .push(channel_id.mention().to_string())
                                .push(" was deleted. its content could not be found in the cache.");
                        }
                    } else {
                        msg.push("a message in ")
                            .push(channel_id.mention().to_string())
                            .push(
                                " was deleted. there is no cache to retrieve message content from.",
                            );
                    }

                    send(
                        *guild_id,
                        embed_with_fields("message deleted", &msg.build(), fields),
                    )
                    .await
                } else {
                    Ok(())
                }
            }

            FullEvent::MessageDeleteBulk {
                channel_id,
                multiple_deleted_messages_ids,
                guild_id,
            } => {
                if let Some(guild_id) = guild_id {
                    send(
                        *guild_id,
                        embed_with_fields(
                            "bulk message deletion executed",
                            &format!(
                                "{} messages were deleted in <#{}>",
                                multiple_deleted_messages_ids.len(),
                                channel_id
                            ),
                            vec![(
                                "deleted message ids".into(),
                                format!("{:?}", multiple_deleted_messages_ids),
                                false,
                            )],
                        ),
                    )
                    .await
                } else {
                    Ok(())
                }
            }

            FullEvent::MessageUpdate {
                old_if_available,
                new,
                event,
            } => handle_message_update(send, old_if_available, new, event).await,

            FullEvent::ReactionRemove { removed_reaction } => {
                if let Some(guild_id) = removed_reaction.guild_id {
                    let mut msg = MessageBuilder::new();

                    msg.push("in ")
                        .push(removed_reaction.channel_id.mention().to_string())
                        .build();

                    match &removed_reaction.emoji {
                        ReactionType::Unicode(emoji) => {
                            msg.push(" with unicode emoji ").push(emoji);
                        }
                        ReactionType::Custom { id, name, .. } => {
                            if let Some(name) = name {
                                msg.push(" with custom emoji ")
                                    .push_bold(name)
                                    .push(" id ")
                                    .push_mono(id.to_string());
                            } else {
                                msg.push(" with custom emoji id ").push_mono(id.to_string());
                            }
                        }
                        _ => {}
                    }

                    if let Some(user_id) = removed_reaction.user_id {
                        msg.push(" from ").push(user_id.mention().to_string());
                    }

                    msg.push("\n");

                    let link = &removed_reaction
                        .message_id
                        .link(removed_reaction.channel_id, Some(guild_id));
                    msg.push_named_link("(go to message)", link);

                    send(guild_id, simple_embed("reaction removed", &msg.build())).await
                } else {
                    Ok(())
                }
            }

            FullEvent::ReactionRemoveAll {
                channel_id,
                removed_from_message_id,
            } => {
                let channel = channel_id
                    .to_channel(&context.http)
                    .await
                    .map_err(|e| DiscordHandlerError::from_display(self.name(), e))?;

                if let Some(guild_channel) = channel.guild() {
                    let link =
                        removed_from_message_id.link(*channel_id, Some(guild_channel.guild_id));
                    let msg = MessageBuilder::new()
                        .push("on message id ")
                        .push_mono(removed_from_message_id.to_string())
                        .push(" in ")
                        .push_line(channel_id.mention().to_string())
                        .push_named_link("(go to message)", link)
                        .build();

                    send(
                        guild_channel.guild_id,
                        simple_embed("all reactions removed", &msg),
                    )
                    .await
                } else {
                    Ok(())
                }
            }

            FullEvent::ReactionRemoveEmoji { removed_reactions } => {
                if let Some(guild_id) = removed_reactions.guild_id {
                    let mut msg = MessageBuilder::new();

                    msg.push("in ")
                        .push(removed_reactions.channel_id.mention().to_string())
                        .build();

                    match &removed_reactions.emoji {
                        ReactionType::Unicode(emoji) => {
                            msg.push(" of unicode emoji ").push(emoji);
                        }
                        ReactionType::Custom { id, name, .. } => {
                            if let Some(name) = name {
                                msg.push(" of custom emoji ")
                                    .push_bold(name)
                                    .push(" id ")
                                    .push_mono(id.to_string());
                            } else {
                                msg.push(" of custom emoji id ").push_mono(id.to_string());
                            }
                        }
                        _ => {}
                    }

                    if let Some(user_id) = removed_reactions.user_id {
                        msg.push(" from ").push(user_id.mention().to_string());
                    }

                    msg.push("\n");

                    let link = &removed_reactions
                        .message_id
                        .link(removed_reactions.channel_id, Some(guild_id));
                    msg.push_named_link("(go to message)", link);

                    send(guild_id, simple_embed("reactions removed", &msg.build())).await
                } else {
                    Ok(())
                }
            }

            FullEvent::VoiceChannelStatusUpdate {
                old,
                status,
                id,
                guild_id,
            } => {
                let old = old
                    .as_ref()
                    .and_then(|old| if old.is_empty() { None } else { Some(old) });

                let new = status.as_ref().and_then(|status| {
                    if status.is_empty() {
                        None
                    } else {
                        Some(status)
                    }
                });

                if old == new {
                    return Ok(());
                }

                let mut msg = MessageBuilder::new();

                msg.push("from ");
                if let Some(old) = old
                    && !old.is_empty()
                {
                    msg.push_bold(old.to_string());
                } else {
                    msg.push("nothing");
                }

                msg.push(" to ");
                if let Some(status) = status
                    && !status.is_empty()
                {
                    msg.push_bold(status.to_string());
                } else {
                    msg.push("nothing");
                }

                msg.push(" in ").push(id.mention().to_string()).build();

                send(
                    *guild_id,
                    simple_embed("voice channel status updated", &msg.build()),
                )
                .await
            }
            _ => Ok(()),
        }
    }
}

impl LoggingHandler {
    const NAME: &'static str = "logging";

    fn map_result<T>(r: anyhow::Result<T>) -> Result<(), DiscordHandlerError> {
        r.map_err(|e| DiscordHandlerError::from_display(Self::NAME, e))?;
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LoggingChannel {
    /// The guild id that owns this logging channel.
    #[serde(skip)]
    guild_id: GuildId,

    /// The actual channel id to which logs will be sent.
    channel_id: ChannelId,
}

const LOGGING_CHANNEL_TABLE: &str = "logging_channel";

#[async_trait]
impl<C: Connection> DbItem<C> for LoggingChannel {
    type GetQuery = GuildId;
    type Id = i64;
    type UpsertContent = Self;

    const NAME: &'static str = LOGGING_CHANNEL_TABLE;

    fn get_id(&self) -> Self::Id {
        self.guild_id.get() as i64
    }

    async fn get_from_db(
        db: &Surreal<C>,
        guild_id: Self::GetQuery,
    ) -> Result<Option<Self>, surrealdb::Error> {
        let mut result = db
            .query("SELECT * FROM $guild_id;")
            .bind(("guild_id", guild_id.get()))
            .await?;

        Ok(result.take::<Option<Self>>(0)?.map(|mut r| {
            r.guild_id = guild_id;
            r
        }))
    }
}

impl LoggingChannel {
    pub fn new(guild_id: GuildId, channel_id: ChannelId) -> Self {
        Self {
            guild_id,
            channel_id,
        }
    }
}

async fn get_logging_channel_for_guild(
    framework: &DiscordFrameworkContext<'_>,
    guild_id: GuildId,
) -> Result<ChannelId, DiscordHandlerError> {
    let db = &framework.user_data().await.db;
    let logging_channel = LoggingChannel::get_from_db(db, guild_id)
        .await
        .map_err(|e| DiscordHandlerError {
            message: format!("error getting logging channel from db: {e}"),
            handler_name: "logging",
        })?;

    if let Some(logging_channel) = logging_channel {
        Ok(logging_channel.channel_id)
    } else {
        Err(DiscordHandlerError {
            message: "no logging channel found for guild".to_string(),
            handler_name: "logging",
        })
    }
}

async fn send_message(
    context: &serenity::Context,
    framework: &DiscordFrameworkContext<'_>,
    guild_id: GuildId,
    message: CreateMessage,
) -> anyhow::Result<()> {
    let logging_channel = get_logging_channel_for_guild(framework, guild_id).await?;
    logging_channel.send_message(&context.http, message).await?;
    Ok(())
}

fn simple_embed(title: &str, message: &str) -> CreateEmbed {
    CreateEmbed::new().title(title).description(message)
}

fn useless_embed(title: &str) -> CreateEmbed {
    simple_embed(title, "muni hasn't bothered to implement useful information for this yet. screenshot this and go bother him.")
}

fn embed_with_fields(
    title: &str,
    message: &str,
    fields: Vec<(String, String, bool)>,
) -> CreateEmbed {
    let mut embed = CreateEmbed::new().title(title);

    if !message.is_empty() {
        embed = embed.description(message);
    }

    if !fields.is_empty() {
        let fields = fields.into_iter().map(|(name, mut value, inline)| {
            if value.len() > 1024 {
                value = value[..1023].to_string();
                value.push('…');
            }
            (name, value, inline)
        });
        embed = embed.fields(fields);
    }

    embed
}

async fn handle_message_update<F, X>(
    send: F,
    old_if_available: &Option<Message>,
    new: &Option<Message>,
    event: &MessageUpdateEvent,
) -> Result<(), DiscordHandlerError>
where
    F: Fn(GuildId, CreateEmbed) -> X,
    X: Future<Output = Result<(), DiscordHandlerError>>,
{
    if let (Some(old), Some(new)) = (old_if_available, new)
        && let Some(guild_id) = event.guild_id
        && old.content != new.content
    {
        let mut msg_builder = MessageBuilder::new();

        // add author mention, if available
        if let Some(author) = &event.author {
            msg_builder.push(format!("from {} ", author.mention()));
        }

        msg_builder
            .push("in ")
            .push_line(event.channel_id.mention().to_string());

        let link = &event.id.link(event.channel_id, event.guild_id);
        msg_builder.push_named_link("(go to message)", link);

        let msg = msg_builder.build().trim().to_owned();

        let fields = vec![
            ("old".into(), old.content.clone(), false),
            ("new".into(), new.content.clone(), false),
        ];

        send(guild_id, embed_with_fields("message edited", &msg, fields)).await
    } else {
        Ok(())
    }
}

async fn handle_member_update<F, X>(
    send: F,
    old_if_available: &Option<Member>,
    new: &Option<Member>,
    event: &GuildMemberUpdateEvent,
) -> Result<(), DiscordHandlerError>
where
    F: Fn(GuildId, CreateEmbed) -> X,
    X: Future<Output = Result<(), DiscordHandlerError>>,
{
    if let (Some(old), Some(new)) = (old_if_available, new) {
        let mut msg = MessageBuilder::new();
        msg.push(event.user.id.mention().to_string())
            .push(" was updated");

        let mut fields: Vec<(&str, String)> = vec![];

        // collect changed fields

        // nickname
        if old.nick != new.nick {
            if let Some(new_nick) = &new.nick {
                fields.push((
                    "nickname",
                    MessageBuilder::new().push_safe(new_nick).build(),
                ))
            } else {
                fields.push((
                    "nickname",
                    MessageBuilder::new().push_italic("removed").build(),
                ))
            }
        }

        // roles
        if old.roles != new.roles {
            let added_roles = new
                .roles
                .iter()
                .filter(|role| !old.roles.contains(role))
                .map(|role| role.mention().to_string())
                .collect::<Vec<String>>();
            let removed_roles = old
                .roles
                .iter()
                .filter(|role| !new.roles.contains(role))
                .map(|role| role.mention().to_string())
                .collect::<Vec<String>>();

            if !added_roles.is_empty() {
                fields.push(("roles added", added_roles.join(" ")))
            }
            if !removed_roles.is_empty() {
                fields.push(("roles removed", removed_roles.join(" ")))
            }
        }

        // timeouts
        if old.communication_disabled_until != new.communication_disabled_until {
            match (
                old.communication_disabled_until,
                new.communication_disabled_until,
            ) {
                (Some(_), None) => fields.push(("timeout removed", String::new())),
                (_, Some(new)) => {
                    fields.push(("timed out", format!("until <t:{}:F>", new.timestamp())))
                }
                _ => {}
            }
        }

        if !fields.is_empty() {
            let fields = fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), value, true))
                .collect();

            send(
                event.guild_id,
                embed_with_fields("member updated", &msg.build(), fields),
            )
            .await?
        }
    }

    Ok(())
}

async fn handle_role_update<F, X>(
    send: F,
    old: &Role,
    new: &Role,
) -> Result<(), DiscordHandlerError>
where
    F: Fn(GuildId, CreateEmbed) -> X,
    X: Future<Output = Result<(), DiscordHandlerError>>,
{
    let mut msg = MessageBuilder::new();
    msg.push("role ")
        .push_bold(new.mention().to_string())
        .push(" was updated");

    let mut fields = vec![];

    if old.name != new.name {
        fields.push(("old name".into(), old.name.to_string(), false));
        fields.push(("new name".into(), new.name.to_string(), false));
    }

    if old.colour != new.colour {
        fields.push(("color".into(), format!("{:#x}", new.colour.0), true));
    }

    if old.hoist != new.hoist {
        fields.push(("hoist".into(), bool_to_string(new.hoist), true));
    }

    if old.mentionable != new.mentionable {
        fields.push(("mentionable".into(), bool_to_string(new.mentionable), true));
    }

    if old.position != new.position {
        fields.push(("position".into(), new.position.to_string(), true));
    }

    if old.permissions != new.permissions {
        let added_perms = (new.permissions - old.permissions)
            .iter_names()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ");
        let removed_perms = (old.permissions - new.permissions)
            .iter_names()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ");

        fields.push(("added permissions".into(), added_perms, false));
        fields.push(("removed permissions".into(), removed_perms, false));
    }

    send(
        new.guild_id,
        embed_with_fields("role updated", &msg.build(), fields),
    )
    .await
}

fn bool_to_string(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}
