#[cfg(feature = "ssr")]
pub mod ssr {
    use muni_bot_core::{config::DbConfig, MuniBotError};
    use surrealdb::{engine::remote::ws, Surreal};

    pub async fn connect_to_db(config: &DbConfig) -> Result<Surreal<ws::Client>, MuniBotError> {
        let db_name = std::env::var("DATABASE_DB_GUI").unwrap_or(config.gui_db_name.clone());
        muni_bot_core::db::connect_to_db(config, &db_name).await
    }
}
