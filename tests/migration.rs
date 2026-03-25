//! Integration test for the SurrealDB -> MySQL migration.
//!
//! SurrealDB runs fully in-process (kv-mem engine) -- no external server
//! required.
//!
//! MySQL still requires the devenv database to be running:
//!   mysql://munibot:sillylittlepassword@127.0.0.1:3306/munibot_test

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use munibot::db::{
    DbPool,
    models::{AutoDeleteTimerRow, GuildConfig, GuildPayout, GuildWallet, Quote},
    schema::{
        autodelete_timers, community_links, guild_configs, guild_payouts, guild_wallets, quotes,
    },
};
use surrealdb::{
    Surreal,
    engine::local::{Db, Mem},
};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// use 127.0.0.1 (not localhost) to force TCP -- the native MySQL C library
// used by diesel's sync MysqlConnection interprets "localhost" as a Unix socket
const TEST_DB_URL: &str = "mysql://munibot:sillylittlepassword@127.0.0.1:3306/munibot_test";

/// Runs all pending Diesel migrations on the test database using a
/// synchronous connection (required by the `MigrationHarness` API).
fn run_diesel_migrations() {
    let mut conn = diesel::MysqlConnection::establish(TEST_DB_URL)
        .expect("couldn't connect to munibot_test for migrations");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("couldn't run diesel migrations on test database");
}

async fn build_test_pool() -> DbPool {
    use diesel_async::{
        AsyncMysqlConnection,
        pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
    };
    let manager = AsyncDieselConnectionManager::<AsyncMysqlConnection>::new(TEST_DB_URL);
    Pool::builder()
        .build(manager)
        .await
        .expect("couldn't build test database pool")
}

/// Creates a fresh in-process SurrealDB instance using the memory engine.
async fn connect_surreal() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(())
        .await
        .expect("couldn't start in-memory surrealdb");
    db.use_ns("test")
        .use_db("test")
        .await
        .expect("couldn't select ns/db");
    db
}

/// Deletes all rows from MySQL test tables in FK-safe order.
async fn truncate_mysql(pool: &DbPool) {
    let mut conn = pool.get().await.expect("couldn't get db connection");
    // quotes references community_links via FK -- must be deleted first
    diesel::delete(quotes::table)
        .execute(&mut conn)
        .await
        .expect("delete quotes");
    diesel::delete(community_links::table)
        .execute(&mut conn)
        .await
        .expect("delete community_links");
    diesel::delete(guild_configs::table)
        .execute(&mut conn)
        .await
        .expect("delete guild_configs");
    diesel::delete(autodelete_timers::table)
        .execute(&mut conn)
        .await
        .expect("delete autodelete_timers");
    diesel::delete(guild_wallets::table)
        .execute(&mut conn)
        .await
        .expect("delete guild_wallets");
    diesel::delete(guild_payouts::table)
        .execute(&mut conn)
        .await
        .expect("delete guild_payouts");
}

/// Seeds SurrealDB with known test data for each migrated table.
async fn seed_surreal(db: &Surreal<Db>) {
    // 2 logging channels: record ID key is the guild_id (integer)
    db.query("CREATE logging_channel:111111111 SET channel_id = 222222222")
        .await
        .expect("seed logging_channel 1");
    db.query("CREATE logging_channel:333333333 SET channel_id = 444444444")
        .await
        .expect("seed logging_channel 2");

    // 2 autodelete timers: duration stored as a humantime string
    db.query(
        "CREATE autodelete_timer SET \
         channel_id = 555555555, guild_id = 111111111, \
         duration = '30m', last_cleaned = time::now(), \
         last_message_id_cleaned = 0, mode = 'after_time'",
    )
    .await
    .expect("seed autodelete_timer 1");
    db.query(
        "CREATE autodelete_timer SET \
         channel_id = 666666666, guild_id = 333333333, \
         duration = '1h', last_cleaned = time::now(), \
         last_message_id_cleaned = 999, mode = 'after_inactivity'",
    )
    .await
    .expect("seed autodelete_timer 2");

    // 2 guild wallets
    db.query("CREATE guild_wallet SET guild_id = 111111111, user_id = 777777777, balance = 100")
        .await
        .expect("seed guild_wallet 1");
    db.query("CREATE guild_wallet SET guild_id = 111111111, user_id = 888888888, balance = 200")
        .await
        .expect("seed guild_wallet 2");

    // 2 guild payouts
    db.query(
        "CREATE guild_payout SET guild_id = 111111111, user_id = 777777777, \
         balance = 50, last_payout = time::now()",
    )
    .await
    .expect("seed guild_payout 1");
    db.query(
        "CREATE guild_payout SET guild_id = 333333333, user_id = 999999999, \
         balance = 75, last_payout = time::now()",
    )
    .await
    .expect("seed guild_payout 2");

    // 3 quotes with explicit created_at timestamps so sequential_id order is
    // deterministic (migration orders by created_at ASC when assigning ids)
    db.query(
        "CREATE quote SET \
         created_at = '2024-01-01T00:00:00Z', quote = 'first quote', \
         invoker = 'user_a', stream_category = 'gaming', stream_title = 'playing stuff'",
    )
    .await
    .expect("seed quote 1");
    db.query(
        "CREATE quote SET \
         created_at = '2024-06-15T12:00:00Z', quote = 'second quote', \
         invoker = 'user_b', stream_category = 'music', stream_title = 'making music'",
    )
    .await
    .expect("seed quote 2");
    db.query(
        "CREATE quote SET \
         created_at = '2024-12-31T23:59:59Z', quote = 'third quote', \
         invoker = 'user_c', stream_category = 'art', stream_title = 'drawing things'",
    )
    .await
    .expect("seed quote 3");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_migration_migrates_and_is_idempotent() {
    let _ = env_logger::try_init();

    // run diesel schema migrations on the test database
    run_diesel_migrations();
    let pool = build_test_pool().await;

    // in-memory surrealdb -- always starts clean, no external server needed
    let surreal = connect_surreal().await;

    // start mysql from a known-clean state in case a previous run failed partway
    truncate_mysql(&pool).await;

    // seed source data into surrealdb
    seed_surreal(&surreal).await;

    // run the migration
    munibot::db::migration::migrate_from_surrealdb(&pool, &surreal)
        .await
        .expect("migration should succeed");

    let mut conn = pool.get().await.unwrap();

    // --- guild_configs ---
    let configs: Vec<GuildConfig> = guild_configs::table
        .order(guild_configs::guild_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(configs.len(), 2, "should have 2 guild_configs");
    assert_eq!(configs[0].guild_id, 111_111_111);
    assert_eq!(configs[0].logging_channel, Some(222_222_222));
    assert_eq!(configs[1].guild_id, 333_333_333);
    assert_eq!(configs[1].logging_channel, Some(444_444_444));

    // --- autodelete_timers ---
    let timers: Vec<AutoDeleteTimerRow> = autodelete_timers::table
        .order(autodelete_timers::channel_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(timers.len(), 2, "should have 2 autodelete_timers");
    assert_eq!(timers[0].channel_id, 555_555_555);
    assert_eq!(timers[0].guild_id, 111_111_111);
    assert_eq!(timers[0].duration_secs, 30 * 60, "30m = 1800s");
    assert_eq!(timers[0].last_message_id_cleaned, 0);
    assert_eq!(timers[0].mode, "after_time");
    assert_eq!(timers[1].channel_id, 666_666_666);
    assert_eq!(timers[1].guild_id, 333_333_333);
    assert_eq!(timers[1].duration_secs, 60 * 60, "1h = 3600s");
    assert_eq!(timers[1].last_message_id_cleaned, 999);
    assert_eq!(timers[1].mode, "after_inactivity");

    // --- guild_wallets ---
    let wallets: Vec<GuildWallet> = guild_wallets::table
        .order(guild_wallets::user_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(wallets.len(), 2, "should have 2 guild_wallets");
    assert_eq!(wallets[0].guild_id, 111_111_111);
    assert_eq!(wallets[0].user_id, 777_777_777);
    assert_eq!(wallets[0].balance, 100);
    assert_eq!(wallets[1].guild_id, 111_111_111);
    assert_eq!(wallets[1].user_id, 888_888_888);
    assert_eq!(wallets[1].balance, 200);

    // --- guild_payouts ---
    let payouts: Vec<GuildPayout> = guild_payouts::table
        .order(guild_payouts::user_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(payouts.len(), 2, "should have 2 guild_payouts");
    assert_eq!(payouts[0].guild_id, 111_111_111);
    assert_eq!(payouts[0].user_id, 777_777_777);
    assert_eq!(payouts[0].balance, 50);
    assert_eq!(payouts[1].guild_id, 333_333_333);
    assert_eq!(payouts[1].user_id, 999_999_999);
    assert_eq!(payouts[1].balance, 75);

    // --- community_links: exactly one default community row ---
    let community_count: i64 = community_links::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    assert_eq!(community_count, 1, "should have 1 community_link");

    // --- quotes: sequential_id assigned in ascending created_at order ---
    let migrated_quotes: Vec<Quote> = quotes::table
        .order(quotes::sequential_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(migrated_quotes.len(), 3, "should have 3 quotes");
    assert_eq!(migrated_quotes[0].sequential_id, 1);
    assert_eq!(migrated_quotes[0].quote, "first quote");
    assert_eq!(migrated_quotes[0].invoker, "user_a");
    assert_eq!(migrated_quotes[0].stream_category, "gaming");
    assert_eq!(migrated_quotes[0].stream_title, "playing stuff");
    assert_eq!(migrated_quotes[1].sequential_id, 2);
    assert_eq!(migrated_quotes[1].quote, "second quote");
    assert_eq!(migrated_quotes[1].invoker, "user_b");
    assert_eq!(migrated_quotes[1].stream_category, "music");
    assert_eq!(migrated_quotes[1].stream_title, "making music");
    assert_eq!(migrated_quotes[2].sequential_id, 3);
    assert_eq!(migrated_quotes[2].quote, "third quote");
    assert_eq!(migrated_quotes[2].invoker, "user_c");
    assert_eq!(migrated_quotes[2].stream_category, "art");
    assert_eq!(migrated_quotes[2].stream_title, "drawing things");
    drop(conn);

    // --- idempotency: second run should detect existing data and skip ---
    munibot::db::migration::migrate_from_surrealdb(&pool, &surreal)
        .await
        .expect("second migration run should succeed (no-op)");

    // counts must be unchanged -- no duplicates
    let mut conn = pool.get().await.unwrap();
    let config_count: i64 = guild_configs::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let quote_count: i64 = quotes::table.count().get_result(&mut conn).await.unwrap();
    assert_eq!(
        config_count, 2,
        "idempotency: guild_configs count unchanged"
    );
    assert_eq!(quote_count, 3, "idempotency: quotes count unchanged");

    // cleanup so the test is re-runnable
    drop(conn);
    truncate_mysql(&pool).await;
}
