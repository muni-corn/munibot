use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use surrealdb::{Connection, RecordIdKey, Surreal, opt::IntoResource};

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
