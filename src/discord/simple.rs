use poise::serenity_prelude::MessageBuilder;

use super::{DiscordContext, commands::DiscordCommandProvider};
use crate::MunibotError;

pub struct SimpleCommandProvider;

impl DiscordCommandProvider for SimpleCommandProvider {
    fn commands(&self) -> Vec<poise::Command<super::state::DiscordState, crate::MunibotError>> {
        vec![tone_indicators()]
    }
}

const MOST_COMMON_TONE_INDICATORS: &[(&str, &str)] = &[
    ("/gen", "genuine"),
    ("/hj", "half-joking"),
    ("/j", "joking"),
    ("/lh", "lighthearted"),
    ("/lyr", "lyrics"),
    ("/nm", "not mad"),
    ("/p", "platonic"),
    ("/sarc", "sarcastic"),
    ("/srs", "serious"),
    ("/t", "teasing"),
];
const UNCOMMON_TONE_INDICATORS: &[(&str, &str)] = &[
    ("/c", "copypasta"),
    ("/nbh", "nobody here targeted"),
    ("/neg", "negative"),
    ("/neu", "neutral"),
    ("/nsrs", "not serious"),
    ("/nsx", "no sexual intent"),
    ("/pos", "positive"),
    ("/q", "quote"),
    ("/r", "romantic"),
    ("/ref", "reference"),
    ("/rh", "rhetorical"),
    ("/sx", "sexual intent"),
    ("/th", "threat"),
];

/// display a guide on tone indicators.
#[poise::command(slash_command, rename = "tone-indicators", ephemeral)]
async fn tone_indicators(ctx: DiscordContext<'_>) -> Result<(), MunibotError> {
    let mut msg = MessageBuilder::new();
    msg
        .push_line("## a guide on tone indicators")
        .push_line("text doesn't have a voice. tone, inflection, and other non-verbal bits of communication are lost through text! to remedy this, we use tone indicators to specify what tone we mean to convey in our messages.")
        .push_line("don't worry about memorizing them all. i'm here to help you remember!")
        .push_line("## common indicators")
        .push_line("these are the tone indicators used most often.");

    for (tag, description) in MOST_COMMON_TONE_INDICATORS {
        msg.push_bold(*tag).push(": ").push_line(*description);
    }

    msg.push_line("## other indicators").push_line(
        "you may not see these tone indicators as much, but they're useful once in a while.",
    );

    for (tag, description) in UNCOMMON_TONE_INDICATORS {
        msg.push_bold(*tag).push(": ").push_line(*description);
    }

    ctx.reply(msg.build()).await?;
    Ok(())
}
