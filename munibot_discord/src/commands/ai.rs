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

pub struct AskCommandProvider;

impl DiscordCommandProvider for AskCommandProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![ask()]
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
    };

    let reply = match ai.turn(request).await {
        Ok(outcome) => outcome.text.unwrap_or_else(|| "...".to_string()),
        Err(error) => format!("something went wrong on my end, sorry :< {error}"),
    };
    let filtered = filter_output(&reply, OutputLimits::new(DISCORD_MESSAGE_LIMIT));

    ctx.say(filtered).await?;
    Ok(())
}
