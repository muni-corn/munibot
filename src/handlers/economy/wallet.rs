use poise::serenity_prelude::{GuildId, UserId};
use serde::{Deserialize, Serialize};
use surrealdb::{Connection, RecordId, Surreal};
use thiserror::Error;

use crate::MunibotError;

pub const GUILD_WALLET_TABLE: &str = "guild_wallet";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletData {
    guild_id: GuildId,
    user_id: UserId,
    balance: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Wallet {
    id: RecordId,

    #[serde(flatten)]
    data: WalletData,
}

impl Wallet {
    /// Retrieves a wallet from the database. If it exists, the existing one is
    /// returned. If it doesn't, a new one is created.
    pub async fn get_from_db<C: Connection>(
        db: &Surreal<C>,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Result<Self, WalletError> {
        match db
            .query(format!(
                "SELECT * FROM {GUILD_WALLET_TABLE}
                 WHERE guild_id = $guild AND user_id = $user;"
            ))
            .bind(("guild", guild_id))
            .bind(("user", user_id))
            .await?
            .take::<Option<Self>>(0)?
        {
            Some(w) => Ok(w),
            None => Self::create_in_db(db, guild_id, user_id, 0).await,
        }
    }

    /// Creates a new wallet in the given database and returns it.
    pub async fn create_in_db<C: Connection>(
        db: &Surreal<C>,
        guild_id: GuildId,
        user_id: UserId,
        balance: u64,
    ) -> Result<Self, WalletError> {
        // insert the wallet into the wallet table
        db.create::<Option<Self>>(GUILD_WALLET_TABLE)
            .content(WalletData {
                guild_id,
                user_id,
                balance,
            })
            .await
            .map_err(WalletError::Database)
            .and_then(|opt| opt.ok_or_else(|| WalletError::NotCreated(user_id, guild_id)))
    }

    /// Updates this wallet in the database.
    async fn update_in_db<C: Connection>(&self, db: &Surreal<C>) -> Result<(), WalletError> {
        let data = self.data.clone();
        db.update::<Option<Self>>((GUILD_WALLET_TABLE, self.id.key().clone()))
            .content(data)
            .await?;
        Ok(())
    }

    /// Deposits the given amount into the wallet and updates it in the
    /// database.
    pub async fn deposit<C: Connection>(
        &mut self,
        db: &Surreal<C>,
        amount: u64,
    ) -> Result<(), WalletError> {
        self.data.balance += amount;
        self.update_in_db(db).await
    }

    /// Spends the given amount from the wallet and updates it in the database.
    pub async fn spend<C: Connection>(
        &mut self,
        db: &Surreal<C>,
        amount: u64,
    ) -> Result<(), WalletError> {
        self.data.balance = self
            .data
            .balance
            .checked_sub(amount)
            .ok_or(WalletError::InsufficientFunds)?;
        self.update_in_db(db).await
    }

    /// The balance of the wallet.
    pub fn balance(&self) -> u64 {
        self.data.balance
    }
}

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("error in wallet database: {0}")]
    Database(#[from] surrealdb::Error),

    #[error("wallet for user {0} in guild {1} not created :<")]
    NotCreated(UserId, GuildId),

    #[error("insufficient funds in wallet")]
    InsufficientFunds,
}

impl From<WalletError> for MunibotError {
    fn from(e: WalletError) -> Self {
        MunibotError::Other(format!("{e}"))
    }
}
