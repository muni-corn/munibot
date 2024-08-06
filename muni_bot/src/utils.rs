use muni_bot_core::{config::DbConfig, MuniBotError};
use surrealdb::{engine::remote::ws, Surreal};

pub async fn connect_to_db(config: &DbConfig) -> Result<Surreal<ws::Client>, MuniBotError> {
    let db_name = std::env::var("DATABASE_DB").unwrap_or(config.db_name.clone());
    muni_bot_core::db::connect_to_db(config, &db_name).await
}
