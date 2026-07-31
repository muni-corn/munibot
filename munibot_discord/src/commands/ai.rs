use munibot_ai::{
    AiTurnRequest,
    memory::ConversationScope,
    persona::{OutputLimits, PersonaId, filter_output},
    tools::Platform,
};
use tokio_util::sync::CancellationToken;

use super::DiscordCommandProvider;
use crate::{
    DiscordCommand, DiscordContext, MunibotDiscordError,
    handlers::ai::{DISCORD_GRANTED_TIER, DISCORD_MESSAGE_LIMIT},
    utils::display_name_from_command_context,
};

pub struct AiCommandProvider;

impl DiscordCommandProvider for AiCommandProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![ask(), persona(), reset()]
    }
}

/// Suggests every configured persona's id whose prefix matches what has been
/// typed so far. Empty when AI isn't configured - `/ask` itself handles that
/// case with a friendly message, so there is nothing useful to suggest here.
async fn autocomplete_persona<'a>(
    ctx: DiscordContext<'_>,
    partial: &'a str,
) -> impl Iterator<Item = String> + 'a {
    let ids: Vec<String> = ctx.data().ai.as_ref().map_or_else(Vec::new, |ai| {
        ai.personas().map(|persona| persona.id.0.clone()).collect()
    });

    ids.into_iter().filter(move |id| id.starts_with(partial))
}

/// Ask munibot something directly, optionally choosing which persona
/// answers.
#[poise::command(slash_command)]
pub async fn ask(
    ctx: DiscordContext<'_>,
    #[description = "what to ask"] prompt: String,
    #[description = "which persona to use"]
    #[autocomplete = "autocomplete_persona"]
    persona: Option<String>,
) -> Result<(), MunibotDiscordError> {
    let Some(ai) = ctx.data().ai.clone() else {
        ctx.say("i'm not set up to chat right now, sorry :<")
            .await?;
        return Ok(());
    };

    let persona_id = persona
        .map(PersonaId::new)
        .or_else(|| ai.default_persona_id().cloned());
    let Some(persona_id) = persona_id else {
        ctx.say("i don't have a default persona configured, so you'll need to pick one :<")
            .await?;
        return Ok(());
    };

    // satisfies discord's three-second interaction deadline for what can be a
    // multi-step tool loop taking much longer than that
    ctx.defer().await?;

    let user_name = display_name_from_command_context(ctx).await;
    let request = AiTurnRequest {
        persona_id,
        scope: ConversationScope::new(Platform::Discord, ctx.channel_id().to_string()),
        user_id: ctx.author().id.get(),
        user_name,
        granted_tier: DISCORD_GRANTED_TIER,
        guild_id: ctx.guild_id().map(|id| id.get()),
        message: prompt,
        cancellation: CancellationToken::new(),
        already_persisted: false,
    };

    let reply = match ai.turn(request).await {
        Ok(outcome) => outcome.text.unwrap_or_else(|| "...".to_string()),
        Err(error) => format!("something went wrong on my end, sorry :< {error}"),
    };
    let filtered = filter_output(&reply, OutputLimits::new(DISCORD_MESSAGE_LIMIT));

    ctx.say(filtered).await?;
    Ok(())
}

/// Shows or pins this channel's persona.
// pinning is in-memory this milestone - it is lost on restart, and becomes a
// real `ai_conversations` column in milestone 2
#[poise::command(slash_command)]
pub async fn persona(
    ctx: DiscordContext<'_>,
    #[description = "pin this channel to a persona (omit to just show the current one)"]
    #[autocomplete = "autocomplete_persona"]
    persona: Option<String>,
) -> Result<(), MunibotDiscordError> {
    let Some(ai) = ctx.data().ai.clone() else {
        ctx.say("i'm not set up to chat right now, sorry :<")
            .await?;
        return Ok(());
    };

    let Some(name) = persona else {
        let pinned = ctx.data().pinned_personas.get(ctx.channel_id()).await;
        let effective = ctx
            .data()
            .pinned_personas
            .effective(ctx.channel_id(), &ai)
            .await;

        let response = match effective.and_then(|id| ai.persona(&id)) {
            Some(resolved) if pinned.is_some() => {
                format!("this channel is pinned to **{}**", resolved.display_name)
            }
            Some(resolved) => format!(
                "using the default persona, **{}**, in this channel",
                resolved.display_name
            ),
            None => "no persona is configured for this channel :<".to_string(),
        };
        ctx.say(response).await?;
        return Ok(());
    };

    let persona_id = PersonaId::new(&name);
    let Some(resolved) = ai.persona(&persona_id) else {
        ctx.say(format!("i don't have a persona named `{name}` :<"))
            .await?;
        return Ok(());
    };
    let display_name = resolved.display_name.clone();

    ctx.data()
        .pinned_personas
        .set(ctx.channel_id(), persona_id)
        .await;
    ctx.say(format!("pinned this channel to **{display_name}** :3"))
        .await?;
    Ok(())
}

/// Clears this channel's conversation with munibot, starting fresh.
#[poise::command(slash_command)]
pub async fn reset(ctx: DiscordContext<'_>) -> Result<(), MunibotDiscordError> {
    let Some(ai) = ctx.data().ai.clone() else {
        ctx.say("i'm not set up to chat right now, sorry :<")
            .await?;
        return Ok(());
    };

    let Some(persona_id) = ctx
        .data()
        .pinned_personas
        .effective(ctx.channel_id(), &ai)
        .await
    else {
        ctx.say("i don't have a default persona configured, so there's nothing to reset :<")
            .await?;
        return Ok(());
    };

    let scope = ConversationScope::new(Platform::Discord, ctx.channel_id().to_string());
    match ai.reset_conversation(&scope, &persona_id).await {
        Ok(()) => {
            ctx.say("okay, i've cleared this channel's conversation with me :3")
                .await?;
        }
        Err(error) => {
            ctx.say(format!("couldn't reset that, sorry :< {error}"))
                .await?;
        }
    }
    Ok(())
}
