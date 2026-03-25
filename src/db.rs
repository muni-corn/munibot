use diesel_async::{
    AsyncMysqlConnection,
    pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
};

pub mod migration;
pub mod models;
pub mod operations;
pub mod schema;

/// Async MySQL connection pool backed by bb8.
///
/// `Pool<C>` from `diesel_async::pooled_connection::bb8` is already
/// `bb8::Pool<AsyncDieselConnectionManager<C>>`, so `C` here is
/// `AsyncMysqlConnection`.
pub type DbPool = Pool<AsyncMysqlConnection>;

/// Creates a new database connection pool using the `DATABASE_URL` environment
/// variable.
pub async fn establish_pool() -> Result<DbPool, Box<dyn std::error::Error + Send + Sync>> {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = AsyncDieselConnectionManager::<AsyncMysqlConnection>::new(&database_url);
    let pool = Pool::builder().build(config).await?;
    Ok(pool)
}
