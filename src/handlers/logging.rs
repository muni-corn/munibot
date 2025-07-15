use std::{
    collections::{HashMap, HashSet},
    future::Future,
};

use log::debug;
use poise::serenity_prelude::{
    self as serenity, CacheHttp, ChannelId, CreateEmbed, CreateMessage, EmbedMessageBuilding,
    FullEvent, Guild, GuildId, GuildMemberUpdateEvent, Member, Mentionable, Message,
    MessageBuilder, MessageId, MessageUpdateEvent, PartialGuild, ReactionType, Result, Role,
    async_trait,
};
use serde::{Deserialize, Serialize};
use surrealdb::{Connection, RecordId, Surreal};

use crate::{
    db::DbItem,
    discord::{
        DiscordFrameworkContext,
        handler::{DiscordEventHandler, DiscordHandlerError},
        state::GlobalAccess,
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PauseType {
    MessageDeleteBulk,
    MessageDelete,
}

#[derive(Debug)]
pub struct LoggingHandler {
    /// channels with pauses on them. a pause prevents logs from being created
    /// about the corresponding `PauseType`.
    pauses: HashMap<ChannelId, HashSet<PauseType>>,

    /// messages to ignore logs for, to be used with things like autodelete to
    /// prevent log spam
    ignored_messages: HashSet<MessageId>,

    access: GlobalAccess,
}

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
                    framework.user_data.access().db(),
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
                            "<t:{account_created_timestamp}:F>, <t:{account_created_timestamp}:R>"
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
                old_data_if_available,
                new_data,
            } => {
                if let Some(old) = old_data_if_available {
                    handle_guild_update(send, old, new_data).await
                } else {
                    Ok(())
                }
            }

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
                // check if we have a guild_id first, as logs can't happen anyway without it
                if let Some(guild_id) = guild_id {
                    let ignored = self.ignored_messages.contains(deleted_message_id);
                    let paused = self
                        .pauses
                        .get(channel_id)
                        .map(|pause_set| pause_set.contains(&PauseType::MessageDelete))
                        .unwrap_or(false);

                    if ignored || paused {
                        // since this message will likely never be seen again, remove it from
                        // ignored messages (if it exists)
                        self.ignored_messages.remove(deleted_message_id);

                        return Ok(());
                    }

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
                                format!("{multiple_deleted_messages_ids:?}"),
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

    pub fn new(access: GlobalAccess) -> Self {
        Self {
            pauses: Default::default(),
            ignored_messages: HashSet::new(),
            access,
        }
    }

    pub async fn send_simple_log(
        &self,
        guild_id: GuildId,
        title: &str,
        message: &str,
    ) -> Result<(), anyhow::Error> {
        let embed = simple_embed(title, message);
        send_message(
            self.access.as_cache_http(),
            self.access.db(),
            guild_id,
            CreateMessage::new().embed(embed),
        )
        .await
    }

    pub async fn set_pauses(
        &mut self,
        channel_id: ChannelId,
        pause_types: &[PauseType],
        reason: &str,
    ) {
        self.pauses
            .insert(channel_id, HashSet::from_iter(pause_types.iter().copied()));

        // alert the guild of the pause, if possible
        if let Ok(Some(guild_id)) = self
            .access
            .http()
            .get_channel(channel_id)
            .await
            .map(|c| c.guild().map(|gc| gc.guild_id))
            && let Err(e) = send_message(
                self.access.as_cache_http(),
                self.access.db(),
                guild_id,
                CreateMessage::new().embed(
                    CreateEmbed::new()
                        .title("logging pauses set")
                        .description(format!("for channel {}", channel_id.mention()))
                        .field("because", reason, false),
                ),
            )
            .await
        {
            log::error!("error while alerting of set_pauses: {e}")
        }
    }

    pub async fn clear_pauses(&mut self, channel_id: ChannelId) {
        self.pauses.remove(&channel_id);

        // alert the guild of the resume, if possible
        if let Ok(Some(guild_id)) = self
            .access
            .http()
            .get_channel(channel_id)
            .await
            .map(|c| c.guild().map(|gc| gc.guild_id))
            && let Err(e) = send_message(
                self.access.as_cache_http(),
                self.access.db(),
                guild_id,
                CreateMessage::new().embed(
                    CreateEmbed::new()
                        .title("logging pauses cleared")
                        .description(format!(
                            "for channel {}. logging has resumed as normal!",
                            channel_id.mention()
                        )),
                ),
            )
            .await
        {
            log::error!("error while alerting of set_pauses: {e}")
        }
    }

    pub fn ignore_message(&mut self, message_id: MessageId) {
        self.ignored_messages.insert(message_id);
    }

    pub fn ignore_messages_iter(&mut self, message_ids: impl Iterator<Item = MessageId>) {
        for m in message_ids {
            self.ignore_message(m);
        }
    }

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
            .query("SELECT * FROM $thing;")
            .bind((
                "thing",
                RecordId::from_table_key(LOGGING_CHANNEL_TABLE, guild_id.get() as i64),
            ))
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

async fn get_logging_channel_for_guild<C: Connection>(
    db: &Surreal<C>,
    guild_id: GuildId,
) -> Result<Option<ChannelId>, DiscordHandlerError> {
    let logging_channel = LoggingChannel::get_from_db(db, guild_id)
        .await
        .map_err(|e| DiscordHandlerError {
            message: format!("error getting logging channel from db: {e}"),
            handler_name: "logging",
        })?;

    Ok(logging_channel.map(|l| l.channel_id))
}

async fn send_message<C: Connection>(
    cache_http: impl CacheHttp,
    db: &Surreal<C>,
    guild_id: GuildId,
    message: CreateMessage,
) -> anyhow::Result<()> {
    if let Some(logging_channel) = get_logging_channel_for_guild(db, guild_id).await? {
        logging_channel.send_message(cache_http, message).await?;
    } else {
        debug!("no logging channel for guild with id {guild_id}")
    }
    Ok(())
}

fn simple_embed(title: &str, message: &str) -> CreateEmbed {
    CreateEmbed::new().title(title).description(message)
}

fn useless_embed(title: &str) -> CreateEmbed {
    simple_embed(
        title,
        "muni hasn't bothered to implement useful information for this yet. screenshot this and go bother him.",
    )
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
        fields.push(("hoist".into(), yes_no_bool(new.hoist), true));
    }

    if old.mentionable != new.mentionable {
        fields.push(("mentionable".into(), yes_no_bool(new.mentionable), true));
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

async fn handle_guild_update<F, X>(
    send: F,
    old: &Guild,
    new: &PartialGuild,
) -> Result<(), DiscordHandlerError>
where
    F: Fn(GuildId, CreateEmbed) -> X,
    X: Future<Output = Result<(), DiscordHandlerError>>,
{
    let mut fields = vec![];

    if old.name != new.name {
        fields.push(("name".to_string(), new.name.clone(), false))
    }

    if old.icon != new.icon {
        let status = if new.icon.is_none() {
            "*removed*"
        } else {
            "*updated*"
        };
        fields.push(("icon".to_string(), status.to_string(), true))
    }

    if old.splash != new.splash {
        fields.push(("splash".to_string(), "updated".to_string(), true))
    }

    if old.verification_level != new.verification_level {
        fields.push((
            "verification level".to_string(),
            format!(
                "was {:?}, now {:?}",
                old.verification_level, new.verification_level
            ),
            false,
        ))
    }

    if old.description != new.description
        && let Some(status) = match (&old.description, &new.description) {
            (Some(_), Some(desc)) | (None, Some(desc)) if !desc.is_empty() => {
                Some(desc.to_string())
            }
            (Some(_), None) => Some("*removed*".to_string()),
            _ => None,
        }
    {
        fields.push(("description".to_string(), status, false))
    };

    if old.banner != new.banner {
        let status = if new.banner.is_none() {
            "*removed*"
        } else {
            "*updated*"
        };
        fields.push(("banner".to_string(), status.to_string(), true))
    }

    if old.discovery_splash != new.discovery_splash {
        let status = if new.discovery_splash.is_none() {
            "*removed*"
        } else {
            "*updated*"
        };
        fields.push(("discovery splash".to_string(), status.to_string(), true))
    }

    if old.premium_tier != new.premium_tier {
        fields.push((
            "premium tier".to_string(),
            format!(
                "was **{:?}**, now **{:?}**",
                old.premium_tier, new.premium_tier
            ),
            true,
        ))
    }

    if old.preferred_locale != new.preferred_locale {
        fields.push((
            "preferred locale".to_string(),
            format!(
                "changed from **{}** to **{}**",
                old.preferred_locale, new.preferred_locale
            ),
            true,
        ))
    }

    if old.default_message_notifications != new.default_message_notifications {
        fields.push((
            "default notifications".to_string(),
            format!(
                "was **{:?}**, now **{:?}**",
                old.default_message_notifications, new.default_message_notifications
            ),
            false,
        ))
    }

    if old.explicit_content_filter != new.explicit_content_filter {
        fields.push((
            "explicit content filter".to_string(),
            format!(
                "was **{:?}**, now **{:?}**",
                old.explicit_content_filter, new.explicit_content_filter
            ),
            false,
        ))
    }

    if old.mfa_level != new.mfa_level {
        fields.push((
            "mfa level".to_string(),
            format!("was **{:?}**, now **{:?}**", old.mfa_level, new.mfa_level),
            true,
        ))
    }

    if old.system_channel_id != new.system_channel_id {
        let status = match (&old.system_channel_id, &new.system_channel_id) {
            (Some(_), None) => "*removed*".to_string(),
            (Some(_), Some(channel)) | (None, Some(channel)) => format!("{}", channel.mention()),
            (None, None) => unreachable!(),
        };
        fields.push(("system channel".to_string(), status, false))
    }

    if old.rules_channel_id != new.rules_channel_id {
        let status = match (&old.rules_channel_id, &new.rules_channel_id) {
            (Some(_), Some(channel)) | (None, Some(channel)) => format!("{}", channel.mention()),
            (Some(_), None) => "*removed*".to_string(),
            (None, None) => unreachable!(),
        };
        fields.push(("rules channel".to_string(), status, false))
    }

    if old.public_updates_channel_id != new.public_updates_channel_id {
        let status = match (
            &old.public_updates_channel_id,
            &new.public_updates_channel_id,
        ) {
            (Some(_), None) => "*removed*".to_string(),
            (Some(_), Some(channel)) | (None, Some(channel)) => format!("{}", channel.mention()),
            (None, None) => unreachable!(),
        };
        fields.push(("public updates channel".to_string(), status, false))
    }

    if old.vanity_url_code != new.vanity_url_code {
        let status = match (&old.vanity_url_code, &new.vanity_url_code) {
            (Some(_), None) => "*removed*".to_string(),
            (Some(_), Some(code)) | (None, Some(code)) => code.to_string(),
            (None, None) => unreachable!(),
        };
        fields.push(("vanity URL".to_string(), status, false))
    }

    if old.owner_id != new.owner_id {
        fields.push((
            "owner".to_string(),
            format!("transferred to **{}**", new.owner_id.mention()),
            false,
        ))
    }

    if old.widget_enabled != new.widget_enabled
        && let (Some(_), Some(enabled)) = (old.widget_enabled, new.widget_enabled)
    {
        fields.push(("widget".to_string(), yes_no_bool(enabled), true))
    }

    if old.widget_channel_id != new.widget_channel_id {
        let status = match (&old.widget_channel_id, &new.widget_channel_id) {
            (Some(_), None) => "*removed*".to_string(),
            (Some(_), Some(channel)) | (None, Some(channel)) => format!("{}", channel.mention()),
            (None, None) => unreachable!(),
        };
        fields.push(("widget channel".to_string(), status, true))
    }

    if old.nsfw_level != new.nsfw_level {
        fields.push((
            "nsfw level".to_string(),
            format!("was {:?}, now {:?}", old.nsfw_level, new.nsfw_level),
            true,
        ))
    }

    if old.premium_progress_bar_enabled != new.premium_progress_bar_enabled {
        fields.push((
            "premium progress bar".to_string(),
            enabled_bool(new.premium_progress_bar_enabled),
            true,
        ))
    }

    // if we don't care about any of the actual changes, quietly do nothing
    if fields.is_empty() {
        Ok(())
    } else {
        send(new.id, embed_with_fields("guild updated", "", fields)).await
    }
}

fn yes_no_bool(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}

fn enabled_bool(b: bool) -> String {
    if b { "enabled" } else { "disabled" }.to_string()
}
