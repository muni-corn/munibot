use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::{auth::Database, IntoResource},
    Connection, RecordIdKey, Surreal,
};

use crate::{config::DbConfig, MuniBotError};

pub async fn connect_to_db(
    config: &DbConfig,
    database_name: &str,
) -> Result<Surreal<Client>, MuniBotError> {
    let db = Surreal::new::<Ws>(&config.url).await?;
    db.signin(Database {
        namespace: "muni_bot",
        database: database_name,
        username: &config.user,
        password: &std::env::var("DATABASE_PASS").expect("expected DATABASE_PASS to be set"),
    })
    .await?;

    Ok(db)
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
