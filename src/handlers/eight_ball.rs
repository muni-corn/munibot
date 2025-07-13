use poise::serenity_prelude::MessageBuilder;
use rand::seq::SliceRandom;

use crate::{
    MuniBotError,
    discord::{
        DiscordCommand, DiscordContext,
        commands::{DiscordCommandError, DiscordCommandProvider},
    },
};

pub struct EightBallProvider;

impl EightBallProvider {
    /// Returns a random shake message, "shakes eight ball <adverb>".
    fn get_shake_message() -> String {
        let mut rng = rand::thread_rng();
        let adverb = SHAKE_ADVERBS.choose(&mut rng).unwrap();
        format!("shakes eight ball {}...", adverb)
    }

    /// Returns a random eight ball response.
    fn get_response() -> &'static str {
        let mut rng = rand::thread_rng();
        EIGHT_BALL_RESPONSES.choose(&mut rng).unwrap()
    }
}

/// Have the bot ask the magic eight ball to predict the future with
/// questionable accuracy!
#[poise::command(prefix_command, track_edits, slash_command)]
async fn eight_ball(
    ctx: DiscordContext<'_>,
    #[description = "A yes-or-no question about the future."] question: String,
) -> Result<(), MuniBotError> {
    let shake_message = EightBallProvider::get_shake_message();
    let eight_ball_response = EightBallProvider::get_response();
    let message = MessageBuilder::new()
        .push_quote_line_safe(question)
        .push_line("")
        .push_italic_line(shake_message)
        .push(format!("the eight ball says, \"{}\"", eight_ball_response))
        .build();

    ctx.say(message).await.map_err(|e| DiscordCommandError {
        message: format!("couldn't send message: {}", e),
        command_identifier: "eight_ball".to_string(),
    })?;

    Ok(())
}

const SHAKE_ADVERBS: [&str; 24] = [
    "anxiously",
    "boldly",
    "briskly",
    "carefully",
    "carelessly",
    "cautiously",
    "curiously",
    "daintily",
    "delicately",
    "doubtfully",
    "eagerly",
    "excitedly",
    "fiercely",
    "firmly",
    "gently",
    "gracefully",
    "impatiently",
    "nervously",
    "recklessly",
    "skeptically",
    "suspiciously",
    "tenderly",
    "vigorously",
    "violently",
];
const EIGHT_BALL_RESPONSES: [&str; 20] = [
    "it is certain!",
    "it is decidedly so!",
    "without a doubt!",
    "yes - definitely!",
    "you may rely on it!",
    "as I see it, yes!",
    "most likely!",
    "outlook good!",
    "yes!",
    "signs point to yes!",
    "reply hazy, try again.",
    "ask again later.",
    "better not tell you now...",
    "cannot predict now.",
    "concentrate and ask again.",
    "don't count on it.",
    "my reply is no.",
    "my sources say no...",
    "outlook not so good...",
    "very doubtful.",
];

impl DiscordCommandProvider for EightBallProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![eight_ball()]
    }
}
