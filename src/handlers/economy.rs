use async_trait::async_trait;
use num_format::{Locale, ToFormattedString};
use poise::serenity_prelude::{Context, FullEvent, Message, UserId};
use wallet::Wallet;

use self::wallet::WalletError;
use crate::{
    MuniBotError,
    discord::{
        DiscordCommand, DiscordContext, DiscordFrameworkContext,
        commands::{DiscordCommandError, DiscordCommandProvider},
        handler::{DiscordEventHandler, DiscordHandlerError},
        utils::display_name_from_command_context,
    },
    handlers::economy::payout::{ClaimResult, Payout, PayoutError},
};

mod payout;
mod wallet;

pub struct EconomyProvider;

impl EconomyProvider {
    fn calc_salary(msg: &Message) -> u64 {
        // determine a salary based on words
        let valid_char_count: i32 = msg
            .content
            .split_whitespace()
            .filter_map(|w| {
                // ignore words containing symbols
                if w.contains(|c: char| !c.is_alphanumeric()) {
                    None
                } else if w.len() > 2 {
                    // pay one unit per character, up to 10 units per word,
                    // and only for words greater than 2 characters
                    Some(w.len().min(10) as i32)
                } else {
                    None
                }
            })
            .sum();

        // use a sigmoid function to curb bigger salaries (to mitigate copypasta spam)
        (2000.0 / (1.0 + 1.002_f64.powi(-valid_char_count))) as u64 - 1000
    }
}

#[async_trait]
impl DiscordEventHandler for EconomyProvider {
    fn name(&self) -> &'static str {
        "economy"
    }

    async fn handle_discord_event(
        &mut self,
        _context: &Context,
        framework: DiscordFrameworkContext<'_>,
        event: &FullEvent,
    ) -> Result<(), DiscordHandlerError> {
        if let FullEvent::Message { new_message } = event {
            let msg = new_message;
            if let Some(guild_id) = msg.guild_id {
                let salary = Self::calc_salary(msg);
                let db = &framework.user_data().await.access().db();

                Payout::get_from_db(db, guild_id, msg.author.id)
                    .await
                    .map_err(|e| DiscordHandlerError {
                        handler_name: self.name(),
                        message: format!("error getting payout from db: {e}"),
                    })?
                    .deposit(db, salary)
                    .await
                    .map_err(|e| DiscordHandlerError {
                        handler_name: self.name(),
                        message: format!("error depositing salary into payout: {e}"),
                    })?;
            }
        }

        Ok(())
    }
}

impl DiscordCommandProvider for EconomyProvider {
    fn commands(&self) -> Vec<DiscordCommand> {
        vec![wallet(), claim(), transfer()]
    }
}

/// check how much money you have.
#[poise::command(slash_command, prefix_command)]
async fn wallet(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    if let Some(guild_id) = ctx.guild_id() {
        let author_name = display_name_from_command_context(ctx).await;

        let db = &ctx.data().access().db();
        let wallet = Wallet::get_from_db(db, guild_id, ctx.author().id).await?;

        // send the wallet balance
        ctx.reply(format!(
            "hey {author_name}! you have **{}** coins in your wallet.",
            wallet.balance().to_formatted_string(&Locale::en)
        ))
        .await?;

        Ok(())
    } else {
        ctx.say("this command can only be used in a server! each server has their own economy. use this command in a server you're in to check your balance there! ^w^")
            .await.map_err(|e| DiscordCommandError {
                message: format!("error sending message: {e}"),
                command_identifier: "wallet".to_string(),
            })?;

        Ok(())
    }
}

/// claim your monies!
#[poise::command(slash_command, prefix_command)]
async fn claim(ctx: DiscordContext<'_>) -> Result<(), MuniBotError> {
    if let Some(guild_id) = ctx.guild_id() {
        let db = &ctx.data().access().db();

        let mut payout = Payout::get_from_db(db, guild_id, ctx.author().id).await?;

        let claim_result = payout.claim_to_wallet(db).await;

        match claim_result {
            Ok(ClaimResult {
                amount_claimed,
                new_balance,
            }) => {
                let author_name = display_name_from_command_context(ctx).await;
                ctx.say(format!(
                    "hey {author_name}, here are **{}** coins! ^w^ you now have **{}** coins.",
                    amount_claimed.to_formatted_string(&Locale::en),
                    new_balance.to_formatted_string(&Locale::en)
                ))
                .await?
            }
            Err(PayoutError::TooSoon) => {
                let timestamp = payout.next_payout_time().timestamp();
                ctx.say(format!(
                    "you can't claim your payout yet! you can claim it again <t:{timestamp}:R>."
                ))
                .await?
            }
            Err(PayoutError::NothingToClaim) => {
                ctx.say("your payout is empty at the moment. try again later!")
                    .await?
            }
            Err(e) => Err(DiscordCommandError {
                message: format!("error claiming payout: {e}"),
                command_identifier: "claim".to_string(),
            })?,
        };
    } else {
        ctx.say("this command can only be used in a server! go claim your payout from me in a server that i share with you ^w^")
            .await?;
    }

    Ok(())
}

/// transfer money to someone else.
#[poise::command(slash_command, prefix_command)]
async fn transfer(
    ctx: DiscordContext<'_>,
    #[description = "the amount you want to send"] amount: u64,
    #[description = "ping who you want to send funds to"] to: UserId,
) -> Result<(), MuniBotError> {
    if let Some(guild_id) = ctx.guild_id() {
        // ensure the author is sending anything at all
        if amount == 0 {
            ctx.say("you've transferred thin air.").await?;

            return Ok(());
        }

        // check if they're trying to transfer to themselves
        if ctx.author().id == to {
            ctx.say("you can't transfer money to yourself! >:(").await?;

            return Ok(());
        }

        // get the author and recipient wallets
        let db = &ctx.data().access().db();
        let mut author_wallet = Wallet::get_from_db(db, guild_id, ctx.author().id).await?;
        let mut recipient_wallet = Wallet::get_from_db(db, guild_id, to).await?;

        // try to spend from the author wallet
        if let Err(e) = author_wallet.spend(db, amount).await {
            match e {
                WalletError::InsufficientFunds => {
                    let message = format!(
                        "you want to transfer **{}** coins, but you only have **{}** coins in your wallet :<",
                        amount.to_formatted_string(&Locale::en),
                        author_wallet.balance().to_formatted_string(&Locale::en)
                    );
                    ctx.say(message).await?;

                    return Ok(());
                }
                _ => {
                    return Err(DiscordCommandError {
                        message: format!("error spending from author wallet: {e}"),
                        command_identifier: "transfer".to_string(),
                    }
                    .into());
                }
            }
        }

        // deposit into the recipient wallet
        recipient_wallet.deposit(db, amount).await?;

        // send a confirmation message
        ctx.say(format!(
            "**{}** coins have been transferred to <@{}>! ^w^ you have **{}** coins left.",
            amount.to_formatted_string(&Locale::en),
            to,
            author_wallet.balance().to_formatted_string(&Locale::en)
        ))
        .await?;

        Ok(())
    } else {
        ctx.say("this command can only be used in a server! visit a server i share with you to transfer coins to someone else ^w^")
            .await?;

        Ok(())
    }
}
