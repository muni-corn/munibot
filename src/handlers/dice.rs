use poise::serenity_prelude::MessageBuilder;
use rand::{Rng, seq::SliceRandom};

use crate::{
    MunibotError,
    discord::{
        DiscordCommand, DiscordContext,
        commands::{DiscordCommandError, DiscordCommandProvider},
    },
};

pub struct DiceHandler;

impl DiceHandler {
    /// Returns a prefix, result, and a suffix for the resulting roll message.
    pub fn roll_for_message(sides: u8) -> RollResult {
        match sides {
            0 => RollResult::SingleMessage("what.".to_string()),
            1 => RollResult::SingleMessage("you roll a one-sided die. it's a 1.".to_string()),
            _ => {
                let result = rand::thread_rng().gen_range(1..=sides);
                if sides == 2 {
                    RollResult::SingleMessage(format!(
                        "coin flip. it's {}!",
                        if result == 1 { "heads" } else { "tails" }
                    ))
                } else {
                    number_to_message(result, sides)
                }
            }
        }
    }
}

/// Roll a die.
#[poise::command(slash_command, prefix_command, track_edits)]
async fn roll(
    ctx: DiscordContext<'_>,
    #[description = "number of sides on the die you want to roll"] sides: u8,
    #[description = "specify what you're rolling for"] purpose: Option<String>,
) -> Result<(), MunibotError> {
    let mut builder = MessageBuilder::new();
    if let Some(p) = purpose {
        builder.push(format!("rolling {p}: "));
    }
    DiceHandler::roll_for_message(sides).add_to_message_builder(&mut builder);

    ctx.say(builder.build())
        .await
        .map_err(|e| DiscordCommandError {
            message: format!("couldn't send message :( {e}"),
            command_identifier: "roll".to_string(),
        })?;
    Ok(())
}

impl DiscordCommandProvider for DiceHandler {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![roll()]
    }
}

const RESULT_PREFIXES: [&str; 5] = [
    "you roll and... it lands on ",
    "...it's a ",
    "you rolled a ",
    "it's a ",
    "",
];

const CRITICAL_FAILURE_SUFFIXES: [&str; 6] = [
    ". ouch.",
    ". better luck next time >.>",
    ". this oughta be good.",
    "... 🍿",
    ". haha. yikes.",
    " lol",
];

const CRITICAL_SUCCESS_SUFFIXES: [&str; 5] =
    ["!! impressive ;3", "!! 🎉🎉", "!! >u<", "!! :D ", "!! >:D "];

fn number_to_message(result: u8, sides: u8) -> RollResult {
    let mut rng = rand::thread_rng();
    let prefix = RESULT_PREFIXES.choose(&mut rng).unwrap();
    match result {
        n if sides < 20 || (n != 1 && n != sides) => {
            let suffix = if n <= sides / 2 { '.' } else { '!' };
            RollResult::Full(prefix.to_string(), n, suffix.to_string())
        }
        1 => RollResult::Full(
            prefix.to_string(),
            result,
            CRITICAL_FAILURE_SUFFIXES
                .choose(&mut rng)
                .unwrap()
                .to_string(),
        ),
        69 => RollResult::Full(prefix.to_string(), result, ". nice ;3".to_string()),
        n if n == sides => RollResult::Full(
            prefix.to_string(),
            n,
            CRITICAL_SUCCESS_SUFFIXES
                .choose(&mut rng)
                .unwrap()
                .to_string(),
        ),
        n => RollResult::Full(
            "i don't know how, but you rolled a ".to_string(),
            n,
            " and i don't know how to handle it. this is probably a bug. tell muni!".to_string(),
        ),
    }
}

pub enum RollResult {
    SingleMessage(String),
    Full(String, u8, String),
}

impl RollResult {
    fn add_to_message_builder(&self, builder: &mut MessageBuilder) {
        match self {
            RollResult::SingleMessage(msg) => builder.push(msg),
            RollResult::Full(prefix, result, suffix) => builder
                .push(prefix)
                .push_bold(result.to_string())
                .push(suffix),
        };
    }
}
