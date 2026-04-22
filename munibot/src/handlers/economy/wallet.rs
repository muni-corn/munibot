use poise::serenity_prelude::{GuildId, UserId};
use thiserror::Error;

use crate::{
    CoreError, MuniBotError,
    db::{DbPool, operations},
};

#[derive(Debug)]
pub struct Wallet {
    id: i64,
    balance: u64,
}

impl Wallet {
    /// Retrieves a wallet from the database. If it exists, the existing one is
    /// returned. If it doesn't, a new one is created.
    pub async fn get_from_db(
        db: &DbPool,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Result<Self, WalletError> {
        let row = operations::get_or_create_wallet(db, guild_id.get() as i64, user_id.get() as i64)
            .await
            .map_err(WalletError::Database)?;

        Ok(Self {
            id: row.id,
            balance: row.balance,
        })
    }

    /// Deposits the given amount into the wallet and updates it in the
    /// database.
    pub async fn deposit(&mut self, db: &DbPool, amount: u64) -> Result<(), WalletError> {
        let updated = operations::deposit_to_wallet(db, self.id, amount)
            .await
            .map_err(WalletError::Database)?;
        self.balance = updated.balance;
        Ok(())
    }

    /// Spends the given amount from the wallet and updates it in the database.
    pub async fn spend(&mut self, db: &DbPool, amount: u64) -> Result<(), WalletError> {
        if amount > self.balance {
            return Err(WalletError::InsufficientFunds);
        }
        let updated = operations::spend_from_wallet(db, self.id, amount)
            .await
            .map_err(WalletError::Database)?;
        self.balance = updated.balance;
        Ok(())
    }

    /// The balance of the wallet.
    pub fn balance(&self) -> u64 {
        self.balance
    }
}

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("error in wallet database: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("wallet for user {0} in guild {1} not created :<")]
    NotCreated(UserId, GuildId),

    #[error("insufficient funds in wallet")]
    InsufficientFunds,
}

impl From<WalletError> for MuniBotError {
    fn from(e: WalletError) -> Self {
        Self::Core(CoreError::Other(format!("{e}")))
    }
}
