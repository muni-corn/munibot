use std::sync::Arc;

use async_trait::async_trait;
use munibot_ai::{
    Ai, AiTurnRequest,
    memory::ConversationScope,
    persona::PersonaId,
    tools::{Platform, RiskTier},
};
use munibot_core::db::operations;
use poise::serenity_prelude::{ChannelId, Context, FullEvent, UserId};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    DiscordFrameworkContext,
    handler::{DiscordEventHandler, DiscordHandlerError},
    handlers::ai::render::render_streamed_reply,
    utils::display_name_from_message,
};

pub mod render;

/// Discord's own hard cap on a single message's content length. Shared with
/// [`render`] and with `commands::ai`, which both need to cap or split
/// against the same limit.
pub(crate) const DISCORD_MESSAGE_LIMIT: usize = 2000;

/// No per-guild or per-role permission tiering exists yet for AI chat - a
/// later milestone's concern. Every Discord invocation is granted the same
/// tier; a persona's own configured budget, not this tier, is the actual
/// safety net against runaway cost (see `docs/plans/ai/overview.md`'s "Abuse
/// and cost" section). Shared with `commands::ai`, for the same reason.
pub(crate) const DISCORD_GRANTED_TIER: RiskTier = RiskTier::NetworkRead;

/// Reacts to a direct mention, a reply to munibot, or a direct message, by
/// running one turn against the configured default persona.
///
/// No business logic of its own: everything about personas, tools, and the
/// model lives in [`munibot_ai`]. This is a thin translation from a serenity
/// event into an [`AiTurnRequest`] and back into a Discord message.
pub struct AiChatHandler {
    ai: Arc<Ai>,
}

impl AiChatHandler {
    pub fn new(ai: Arc<Ai>) -> Self {
        Self { ai }
    }
}

#[async_trait]
impl DiscordEventHandler for AiChatHandler {
    fn name(&self) -> &'static str {
        "ai_chat"
    }

    async fn handle_discord_event(
        &mut self,
        context: &Context,
        framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        let FullEvent::Message { new_message: msg } = event else {
            return Ok(());
        };

        let bot_id = context.cache.current_user().id;
        if msg.author.id == bot_id {
            return Ok(());
        }

        // a cheap, in-memory check first - no database or provider call happens
        // unless the message is actually meant for the bot
        let is_dm = msg.guild_id.is_none();
        let is_reply_to_bot = msg
            .referenced_message
            .as_ref()
            .is_some_and(|referenced| referenced.author.id == bot_id);
        let is_mentioned = msg.mentions_user_id(bot_id);
        if !is_dm && !is_reply_to_bot && !is_mentioned {
            return Ok(());
        }

        let db = framework.user_data().await.access().db().clone();

        // per-guild gating (milestone 6 phase 23): a dm has no guild_id and is
        // never subject to any of this - a guild's own settings only ever
        // govern that guild's own channels, never munibot's dms
        let mut guild_default_persona = None;
        if let Some(guild_id) = msg.guild_id {
            let guild_id_i64 = guild_id.get() as i64;
            let config = operations::get_guild_config(&db, guild_id_i64)
                .await
                .map_err(|error| DiscordHandlerError::from_display(self.name(), error))?;

            if !config.as_ref().is_some_and(|config| config.ai_enabled) {
                return Ok(());
            }

            if config
                .as_ref()
                .map(|config| config.ai_channel_mode.as_str())
                == Some("allowlist")
            {
                let allowed = allowed_by_channel_allowlist(&db, guild_id_i64, msg.channel_id)
                    .await
                    .map_err(|error| DiscordHandlerError::from_display(self.name(), error))?;
                if !allowed {
                    return Ok(());
                }
            }

            guild_default_persona = config.and_then(|config| config.ai_default_persona);
        }

        let pinned_personas = &framework.user_data().await.pinned_personas;
        let persona_id = match pinned_personas.get(msg.channel_id).await {
            Some(pinned) => Some(pinned),
            // a guild's own default (set on its ai settings page) takes
            // precedence over the service-wide one, but never over an
            // explicit per-channel pin above
            None => guild_default_persona
                .map(PersonaId::new)
                .or_else(|| self.ai.default_persona_id().cloned()),
        };
        let Some(persona_id) = persona_id else {
            warn!("ai chat triggered, but no default_persona is configured");
            msg.channel_id
                .say(
                    &context.http,
                    "i don't have a default persona configured to chat with, sorry :< ask whoever \
                     runs me to set ai.default_persona",
                )
                .await
                .map_err(|error| DiscordHandlerError::from_display(self.name(), error))?;
            return Ok(());
        };

        let user_name = display_name_from_message(msg, &context.http).await;
        let content = strip_leading_mention(&msg.content, bot_id);

        let request = AiTurnRequest {
            persona_id,
            scope: ConversationScope::new(Platform::Discord, msg.channel_id.to_string()),
            // the raw platform snowflake, not a resolved internal `users.id` - no
            // per-user memory exists yet to need the real one (see milestone 2
            // phase 10), and resolving it now would mean fabricating placeholder
            // oauth fields into `linked_accounts` for a lookup this milestone has
            // no other use for
            user_id: msg.author.id.get(),
            user_name,
            granted_tier: DISCORD_GRANTED_TIER,
            guild_id: msg.guild_id.map(|id| id.get()),
            message: content.to_string(),
            cancellation: CancellationToken::new(),
            already_persisted: false,
        };

        // a failed turn gets a friendly in-channel reply rather than propagating: the
        // dispatch loop that calls this handler only logs a propagated error, and a
        // user watching the channel deserves to know something happened at all
        if let Err(error) =
            render_streamed_reply(&context.http, msg.channel_id, &self.ai, request).await
        {
            warn!(%error, "ai turn failed to start");
            msg.channel_id
                .say(&context.http, "something went wrong on my end, sorry :<")
                .await
                .map_err(|error| DiscordHandlerError::from_display(self.name(), error))?;
        }

        Ok(())
    }
}

/// Strips a literal leading mention of `bot_id` (`<@id>` or `<@!id>`) and any
/// whitespace after it, so the model sees "what's the weather" rather than
/// "<@123456789012345678> what's the weather" - Discord clients insert this
/// token automatically when a message starts with typing the bot's name.
/// Whether `channel_id` is in `guild_id`'s ai channel allowlist - only ever
/// consulted when that guild's `ai_channel_mode` is `"allowlist"` in the
/// first place; a guild left on the default `"all"` mode never reaches
/// this at all.
async fn allowed_by_channel_allowlist(
    db: &munibot_core::db::DbPool,
    guild_id: i64,
    channel_id: ChannelId,
) -> diesel::QueryResult<bool> {
    let allowlist = operations::ai::list_ai_channel_allowlist(db, guild_id).await?;
    Ok(allowlist.contains(&(channel_id.get() as i64)))
}

fn strip_leading_mention(content: &str, bot_id: UserId) -> &str {
    let with_bang = format!("<@!{bot_id}>");
    let without_bang = format!("<@{bot_id}>");

    let stripped = content
        .strip_prefix(&with_bang)
        .or_else(|| content.strip_prefix(&without_bang));

    stripped.map_or(content, str::trim_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_leading_mention_removes_the_plain_form() {
        let bot_id = UserId::new(123456789012345678);
        let content = "<@123456789012345678> what's the weather";
        assert_eq!(strip_leading_mention(content, bot_id), "what's the weather");
    }

    #[test]
    fn test_strip_leading_mention_removes_the_nickname_form() {
        let bot_id = UserId::new(123456789012345678);
        let content = "<@!123456789012345678> hello there";
        assert_eq!(strip_leading_mention(content, bot_id), "hello there");
    }

    #[test]
    fn test_strip_leading_mention_leaves_unrelated_text_untouched() {
        let bot_id = UserId::new(123456789012345678);
        let content = "hey does anyone know the weather";
        assert_eq!(strip_leading_mention(content, bot_id), content);
    }

    #[test]
    fn test_strip_leading_mention_ignores_a_different_users_mention() {
        let bot_id = UserId::new(123456789012345678);
        let content = "<@999999999999999999> what's the weather";
        assert_eq!(strip_leading_mention(content, bot_id), content);
    }

    #[test]
    fn test_strip_leading_mention_only_strips_a_leading_mention_not_a_trailing_one() {
        let bot_id = UserId::new(123456789012345678);
        let content = "hey <@123456789012345678> what's the weather";
        assert_eq!(strip_leading_mention(content, bot_id), content);
    }
}
