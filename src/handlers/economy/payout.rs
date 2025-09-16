use chrono::Local;
use poise::serenity_prelude::{GuildId, UserId};
use serde::{Deserialize, Serialize};
use surrealdb::{Connection, RecordId, Surreal};
use thiserror::Error;

use super::wallet::{Wallet, WalletError};
use crate::MunibotError;

const GUILD_PAYOUT_TABLE: &str = "guild_payout";

const PAYOUT_INTERVAL: chrono::Duration = chrono::Duration::milliseconds(1000 * 60 * 5);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PayoutData {
    guild_id: GuildId,
    user_id: UserId,
    balance: u64,
    last_payout: chrono::DateTime<Local>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Payout {
    id: RecordId,

    #[serde(flatten)]
    data: PayoutData,
}

pub struct ClaimResult {
    pub amount_claimed: u64,
    pub new_balance: u64,
}

impl Payout {
    /// Retrieves a payout entry from the database. If it exists, the existing
    /// one is returned. If it doesn't, a new one is created.
    pub async fn get_from_db<C: Connection>(
        db: &Surreal<C>,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Result<Self, PayoutError> {
        match db
            .query(format!(
                "SELECT * FROM {GUILD_PAYOUT_TABLE}
                 WHERE guild_id = $guild AND user_id = $user;"
            ))
            .bind(("guild", guild_id))
            .bind(("user", user_id))
            .await?
            .take::<Option<Self>>(0)?
        {
            Some(w) => Ok(w),
            None => Self::create_in_db(db, guild_id, user_id).await,
        }
    }

    /// Creates a new payout in the given database and returns it.
    async fn create_in_db<C: Connection>(
        db: &Surreal<C>,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Result<Self, PayoutError> {
        // insert the payout into the payout table
        db.create::<Option<Self>>(GUILD_PAYOUT_TABLE)
            .content(PayoutData {
                guild_id,
                user_id,
                balance: 0,
                last_payout: chrono::Local::now() - PAYOUT_INTERVAL,
            })
            .await
            .map_err(PayoutError::Database)
            .and_then(|opt| opt.ok_or_else(|| PayoutError::NotCreated(user_id, guild_id)))
    }

    /// Updates this payout in the database.
    async fn update_in_db<C: Connection>(&self, db: &Surreal<C>) -> Result<(), PayoutError> {
        let data = self.data.clone();
        db.update::<Option<Self>>((GUILD_PAYOUT_TABLE, self.id.key().clone()))
            .content(data)
            .await?;
        Ok(())
    }

    /// Drains the payout into the corresponding user's guild wallet. Returns
    /// the amount claimed as a receipt.
    pub async fn claim_to_wallet<C: Connection>(
        &mut self,
        db: &Surreal<C>,
    ) -> Result<ClaimResult, PayoutError> {
        if chrono::Local::now() < self.next_payout_time() {
            Err(PayoutError::TooSoon)
        } else if self.data.balance == 0 {
            Err(PayoutError::NothingToClaim)
        } else {
            let mut wallet = Wallet::get_from_db(db, self.data.guild_id, self.data.user_id).await?;

            // deposit the payout into the user's wallet
            let amount_claimed = self.data.balance;
            wallet.deposit(db, self.data.balance).await?;

            // clear this payout
            self.data.balance = 0;
            self.data.last_payout = chrono::Local::now();
            self.update_in_db(db).await?;

            Ok(ClaimResult {
                amount_claimed,
                new_balance: wallet.balance(),
            })
        }
    }

    /// Returns the time at which a user can claim their payout.
    pub fn next_payout_time(&self) -> chrono::DateTime<Local> {
        self.data.last_payout + PAYOUT_INTERVAL
    }

    /// Adds the given amount to the pending payout.
    pub async fn deposit<C: Connection>(
        &mut self,
        db: &Surreal<C>,
        amount: u64,
    ) -> Result<(), PayoutError> {
        self.data.balance += amount;
        self.update_in_db(db).await
    }
}

#[derive(Error, Debug)]
pub enum PayoutError {
    #[error("error in payout database: {0}")]
    Database(#[from] surrealdb::Error),

    #[error("error with wallet: {0}")]
    Wallet(#[from] WalletError),

    #[error("payout for user {0} in guild {1} not created :<")]
    NotCreated(UserId, GuildId),

    #[error("too soon to claim! wait some time before claiming again")]
    TooSoon,

    #[error("nothing to claim!")]
    NothingToClaim,
}

impl From<PayoutError> for MunibotError {
    fn from(e: PayoutError) -> Self {
        MunibotError::Other(format!("{e}"))
    }
}
