use async_trait::async_trait;
use diesel_async::{
    AsyncMysqlConnection,
    pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
};
use serde::{Serialize, de::DeserializeOwned};
use surrealdb::{Connection, RecordIdKey, Surreal, opt::IntoResource};

pub mod models;
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

#[async_trait]
pub trait DbItem<C: Connection>: Serialize + DeserializeOwned {
    const NAME: &'static str;
    type Id: Into<RecordIdKey>;
    type GetQuery;
    type UpsertContent: Serialize + Send + 'static;

    fn get_id(&self) -> Self::Id;

    fn as_into_resource(&self) -> impl IntoResource<Option<Self>> {
        (Self::NAME, self.get_id())
    }

    async fn get_from_db(
        db: &Surreal<C>,
        query: Self::GetQuery,
    ) -> Result<Option<Self>, surrealdb::Error>;

    async fn upsert_in_db<'a>(
        &self,
        db: &'a Surreal<C>,
        content: Self::UpsertContent,
    ) -> Result<Option<Self>, surrealdb::Error> {
        db.upsert(self.as_into_resource()).content(content).await
    }

    async fn delete_from_db(&self, db: &Surreal<C>) -> Result<Option<Self>, surrealdb::Error> {
        db.delete(self.as_into_resource()).await
    }
}
