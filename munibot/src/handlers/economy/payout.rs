use chrono::{Local, NaiveDateTime, Utc};
use poise::serenity_prelude::{GuildId, UserId};
use thiserror::Error;

use super::wallet::{Wallet, WalletError};
use crate::{
    MuniBotError,
    db::{DbPool, operations},
};

const PAYOUT_INTERVAL: chrono::Duration = chrono::Duration::milliseconds(1000 * 60 * 5);

#[derive(Debug)]
pub struct Payout {
    id: i64,
    guild_id: GuildId,
    user_id: UserId,
    balance: u64,
    last_payout: NaiveDateTime,
}

pub struct ClaimResult {
    pub amount_claimed: u64,
    pub new_balance: u64,
}

impl Payout {
    /// Retrieves a payout entry from the database. If it exists, the existing
    /// one is returned. If it doesn't, a new one is created.
    pub async fn get_from_db(
        db: &DbPool,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Result<Self, PayoutError> {
        // set initial last_payout to one interval in the past so the user can
        // claim immediately after joining
        let initial_last_payout = (Utc::now() - PAYOUT_INTERVAL).naive_utc();

        let row = operations::get_or_create_payout(
            db,
            guild_id.get() as i64,
            user_id.get() as i64,
            initial_last_payout,
        )
        .await
        .map_err(PayoutError::Database)?;

        Ok(Self {
            id: row.id,
            guild_id,
            user_id,
            balance: row.balance,
            last_payout: row.last_payout,
        })
    }

    /// Drains the payout into the corresponding user's guild wallet. Returns
    /// the amount claimed as a receipt.
    pub async fn claim_to_wallet(&mut self, db: &DbPool) -> Result<ClaimResult, PayoutError> {
        if Local::now().naive_local() < self.next_payout_time().naive_local() {
            Err(PayoutError::TooSoon)
        } else if self.balance == 0 {
            Err(PayoutError::NothingToClaim)
        } else {
            let mut wallet = Wallet::get_from_db(db, self.guild_id, self.user_id).await?;

            // deposit the payout into the user's wallet
            let amount_claimed = self.balance;
            wallet.deposit(db, self.balance).await?;

            // clear this payout
            let claimed_at = Utc::now().naive_utc();
            operations::claim_payout(db, self.id, claimed_at)
                .await
                .map_err(PayoutError::Database)?;
            self.balance = 0;
            self.last_payout = claimed_at;

            Ok(ClaimResult {
                amount_claimed,
                new_balance: wallet.balance(),
            })
        }
    }

    /// Returns the time at which a user can claim their payout.
    pub fn next_payout_time(&self) -> chrono::DateTime<Local> {
        self.last_payout.and_utc().with_timezone(&Local) + PAYOUT_INTERVAL
    }

    /// Adds the given amount to the pending payout.
    pub async fn deposit(&mut self, db: &DbPool, amount: u64) -> Result<(), PayoutError> {
        self.balance += amount;
        operations::update_payout(db, self.id, self.balance)
            .await
            .map_err(PayoutError::Database)?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum PayoutError {
    #[error("error in payout database: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("error with wallet: {0}")]
    Wallet(#[from] WalletError),

    #[error("payout for user {0} in guild {1} not created :<")]
    NotCreated(UserId, GuildId),

    #[error("too soon to claim! wait some time before claiming again")]
    TooSoon,

    #[error("nothing to claim!")]
    NothingToClaim,
}

impl From<PayoutError> for MuniBotError {
    fn from(e: PayoutError) -> Self {
        MuniBotError::Other(format!("{e}"))
    }
}
