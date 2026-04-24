//! Shared test utilities for integration tests.
//!
//! MySQL requires the devenv database to be running. Two users are used:
//!   root         -- global CREATE/DROP to manage per-test databases
//!   munibot_test -- ALL PRIVILEGES on `munibot_test_%` for table operations
//!
//! These credentials are only and SHOULD ONLY be used for local development
//! servers.

use diesel::prelude::*;
use diesel_async::{
    AsyncMysqlConnection,
    pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use munibot::db::DbPool;
use rand::Rng;

// use 127.0.0.1 (not localhost) to force TCP -- the native MySQL C library
// used by diesel's sync MysqlConnection interprets "localhost" as a Unix socket
pub const ROOT_DB_URL: &str = "mysql://root:sillylittlepassword@127.0.0.1:3306/mysql";
pub const TEST_DB_BASE_URL: &str = "mysql://munibot_test:sillylittlepassword@127.0.0.1:3306";

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// Owns a temporary MySQL database for the duration of a single test.
///
/// The database is created on construction (with a random name to guarantee
/// isolation), all Diesel migrations are applied immediately, and the database
/// is dropped automatically when this value is dropped -- even if the test
/// panics.
pub struct TestDb {
    db_name: String,
    pub pool: DbPool,
}

impl TestDb {
    /// Creates a new isolated test database with all migrations applied.
    pub async fn new() -> Self {
        // generate a random 12-char alphanumeric suffix to ensure uniqueness
        // across concurrent test processes
        let suffix: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(12)
            .map(char::from)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let db_name = format!("munibot_test_{suffix}");

        // create the database via a sync management connection
        {
            let mut conn = MysqlConnection::establish(ROOT_DB_URL)
                .expect("couldn't connect to mysql for test db creation");
            diesel::RunQueryDsl::execute(
                diesel::sql_query(format!("CREATE DATABASE `{db_name}`")),
                &mut conn,
            )
            .expect("couldn't create per-test database");
        }

        // run diesel migrations on the new database
        {
            let db_url = format!("{TEST_DB_BASE_URL}/{db_name}");
            let mut conn = MysqlConnection::establish(&db_url)
                .expect("couldn't connect to per-test database for migrations");
            conn.run_pending_migrations(MIGRATIONS)
                .expect("couldn't run diesel migrations on per-test database");
        }

        // build an async pool pointing at the new database
        let pool = {
            let db_url = format!("{TEST_DB_BASE_URL}/{db_name}");
            let manager = AsyncDieselConnectionManager::<AsyncMysqlConnection>::new(db_url);
            Pool::builder()
                .build(manager)
                .await
                .expect("couldn't build per-test database pool")
        };

        Self { db_name, pool }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // drop the database using a fresh sync connection -- this runs even on
        // test panic so we don't leave stale databases behind
        let mut conn = MysqlConnection::establish(ROOT_DB_URL)
            .expect("couldn't connect to mysql for test db cleanup");
        diesel::RunQueryDsl::execute(
            diesel::sql_query(format!("DROP DATABASE IF EXISTS `{}`", self.db_name)),
            &mut conn,
        )
        .expect("couldn't drop per-test database");
    }
}
