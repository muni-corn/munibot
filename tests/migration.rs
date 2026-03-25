//! Integration test for the SurrealDB -> MySQL migration.
//!
//! SurrealDB runs fully in-process (kv-mem engine) -- no external server
//! required.
//!
//! MySQL still requires the devenv database to be running. Two users are used:
//!   munibot      -- global CREATE/DROP to manage per-test databases
//!   munibot_test -- ALL PRIVILEGES on `munibot_test_%` for table operations

use chrono::{NaiveDateTime, Timelike, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use munibot::db::{
    DbPool,
    models::{AutoDeleteTimerRow, CommunityLink, GuildConfig, GuildPayout, GuildWallet, Quote},
    schema::{
        autodelete_timers, community_links, guild_configs, guild_payouts, guild_wallets, quotes,
    },
};
use rand::Rng;
use surrealdb::{
    Surreal,
    engine::local::{Db, Mem},
};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// use 127.0.0.1 (not localhost) to force TCP -- the native MySQL C library
// used by diesel's sync MysqlConnection interprets "localhost" as a Unix socket
//
// root has global CREATE/DROP to manage per-test databases.
// munibot_test has ALL PRIVILEGES on the `munibot_test_%` wildcard pattern,
// so it is used for all table-level operations (migrations, pool connections).
//
// don't panic: these credentials are only and SHOULD ONLY be used for local
// development servers.
const ROOT_DB_URL: &str = "mysql://root:sillylittlepassword@127.0.0.1:3306/mysql";
const TEST_DB_BASE_URL: &str = "mysql://munibot_test:sillylittlepassword@127.0.0.1:3306";

/// Owns a temporary MySQL database for the duration of a single test.
///
/// The database is created on construction (with a random name to guarantee
/// isolation), all Diesel migrations are applied immediately, and the database
/// is dropped automatically when this value is dropped -- even if the test
/// panics.
struct TestDb {
    db_name: String,
    pub pool: DbPool,
}

impl TestDb {
    async fn new() -> Self {
        // generate a random 12-char hex suffix to make the name unique across
        // concurrent nextest processes
        let suffix: String = rand::thread_rng()
            .sample_iter(rand::distributions::Alphanumeric)
            .take(12)
            .map(char::from)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let db_name = format!("munibot_test_{suffix}");

        // create the database via a sync management connection
        {
            let mut conn = diesel::MysqlConnection::establish(ROOT_DB_URL)
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
            let mut conn = diesel::MysqlConnection::establish(&db_url)
                .expect("couldn't connect to per-test database for migrations");
            conn.run_pending_migrations(MIGRATIONS)
                .expect("couldn't run diesel migrations on per-test database");
        }

        // build an async pool pointing at the new database
        let pool = {
            use diesel_async::{
                AsyncMysqlConnection,
                pooled_connection::{AsyncDieselConnectionManager, bb8::Pool},
            };
            let db_url = format!("{TEST_DB_BASE_URL}/{db_name}");
            let manager = AsyncDieselConnectionManager::<AsyncMysqlConnection>::new(db_url);
            Pool::builder()
                .build(manager)
                .await
                .expect("couldn't build per-test database pool")
        };

        TestDb { db_name, pool }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // drop the database using a fresh sync connection -- this runs even on
        // test panic so we don't leave stale databases behind
        let mut conn = diesel::MysqlConnection::establish(ROOT_DB_URL)
            .expect("couldn't connect to mysql for test db cleanup");
        diesel::RunQueryDsl::execute(
            diesel::sql_query(format!("DROP DATABASE IF EXISTS `{}`", self.db_name)),
            &mut conn,
        )
        .expect("couldn't drop per-test database");
    }
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

/// Seeds SurrealDB with known test data for each migrated table.
async fn seed_surreal(db: &Surreal<Db>) {
    // 2 logging channels: record ID key is the guild_id (integer)
    db.query("CREATE logging_channel:111111111 SET channel_id = 222222222")
        .await
        .expect("seed logging_channel 1");
    db.query("CREATE logging_channel:333333333 SET channel_id = 444444444")
        .await
        .expect("seed logging_channel 2");

    // 3 autodelete timers: varied duration formats to exercise humantime parsing
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
    // 1d12h exercises a compound duration (days + hours) through humantime
    db.query(
        "CREATE autodelete_timer SET \
         channel_id = 777777777, guild_id = 333333333, \
         duration = '1d12h', last_cleaned = time::now(), \
         last_message_id_cleaned = 42, mode = 'after_time'",
    )
    .await
    .expect("seed autodelete_timer 3 (compound duration)");

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
    let db = TestDb::new().await;

    // in-memory surrealdb -- always starts clean, no external server needed
    let surreal = connect_surreal().await;

    // seed source data into surrealdb
    seed_surreal(&surreal).await;

    // record the time just before migration for timestamp window assertions.
    // MySQL datetime has second-level precision, so stored timestamps are
    // truncated to whole seconds. floor before_migration to the second so that
    // a stored value of e.g. 16:18:29.000 still satisfies >= 16:18:29.000
    // (rather than failing against a sub-second before_migration like
    // 16:18:29.351).
    let before_migration = Utc::now()
        .naive_utc()
        .with_nanosecond(0)
        .expect("zero nanoseconds is always valid");

    // run the migration
    munibot::db::migration::migrate_from_surrealdb(&db.pool, &surreal)
        .await
        .expect("migration should succeed");

    // after_migration does not need flooring -- we only need last_cleaned <= it,
    // and a truncated timestamp is always <= the real current time
    let after_migration = Utc::now().naive_utc();

    let mut conn = db.pool.get().await.unwrap();

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
    assert_eq!(timers.len(), 3, "should have 3 autodelete_timers");
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
    assert_eq!(timers[2].channel_id, 777_777_777);
    assert_eq!(timers[2].guild_id, 333_333_333);
    assert_eq!(timers[2].duration_secs, (24 + 12) * 3600, "1d12h = 129600s");
    assert_eq!(timers[2].last_message_id_cleaned, 42);
    assert_eq!(timers[2].mode, "after_time");

    // last_cleaned should be a recent timestamp from time::now() at seed time
    for timer in &timers {
        assert!(
            timer.last_cleaned >= before_migration && timer.last_cleaned <= after_migration,
            "timer {}: last_cleaned {:?} not within migration window [{:?}, {:?}]",
            timer.channel_id,
            timer.last_cleaned,
            before_migration,
            after_migration,
        );
    }

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

    // last_payout should be a recent timestamp from time::now() at seed time
    for payout in &payouts {
        assert!(
            payout.last_payout >= before_migration && payout.last_payout <= after_migration,
            "payout (guild={}, user={}): last_payout {:?} not within migration window",
            payout.guild_id,
            payout.user_id,
            payout.last_payout,
        );
    }

    // --- community_links: exactly one default community row with known content ---
    let community_links_rows: Vec<CommunityLink> =
        community_links::table.load(&mut conn).await.unwrap();
    assert_eq!(
        community_links_rows.len(),
        1,
        "should have 1 community_link"
    );
    let default_community = &community_links_rows[0];
    assert_eq!(
        default_community.twitch_streamer_id.as_deref(),
        Some("590712444"),
        "default community should have the expected twitch streamer id"
    );
    assert_eq!(
        default_community.discord_guild_id, None,
        "default community should have no discord guild id"
    );

    // --- quotes: sequential_id assigned in ascending created_at order ---
    let migrated_quotes: Vec<Quote> = quotes::table
        .order(quotes::sequential_id.asc())
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(migrated_quotes.len(), 3, "should have 3 quotes");

    // every quote must belong to the default community
    for q in &migrated_quotes {
        assert_eq!(
            q.community_id, default_community.id,
            "quote {} must reference the default community",
            q.sequential_id
        );
    }

    // verify field values and that created_at timestamps survived the
    // DateTime<Utc> -> NaiveDateTime conversion correctly
    let expected_timestamps = [
        NaiveDateTime::parse_from_str("2024-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        NaiveDateTime::parse_from_str("2024-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        NaiveDateTime::parse_from_str("2024-12-31 23:59:59", "%Y-%m-%d %H:%M:%S").unwrap(),
    ];
    assert_eq!(migrated_quotes[0].sequential_id, 1);
    assert_eq!(migrated_quotes[0].quote, "first quote");
    assert_eq!(migrated_quotes[0].invoker, "user_a");
    assert_eq!(migrated_quotes[0].stream_category, "gaming");
    assert_eq!(migrated_quotes[0].stream_title, "playing stuff");
    assert_eq!(
        migrated_quotes[0].created_at, expected_timestamps[0],
        "quote 1 created_at mismatch"
    );
    assert_eq!(migrated_quotes[1].sequential_id, 2);
    assert_eq!(migrated_quotes[1].quote, "second quote");
    assert_eq!(migrated_quotes[1].invoker, "user_b");
    assert_eq!(migrated_quotes[1].stream_category, "music");
    assert_eq!(migrated_quotes[1].stream_title, "making music");
    assert_eq!(
        migrated_quotes[1].created_at, expected_timestamps[1],
        "quote 2 created_at mismatch"
    );
    assert_eq!(migrated_quotes[2].sequential_id, 3);
    assert_eq!(migrated_quotes[2].quote, "third quote");
    assert_eq!(migrated_quotes[2].invoker, "user_c");
    assert_eq!(migrated_quotes[2].stream_category, "art");
    assert_eq!(migrated_quotes[2].stream_title, "drawing things");
    assert_eq!(
        migrated_quotes[2].created_at, expected_timestamps[2],
        "quote 3 created_at mismatch"
    );
    drop(conn);

    // --- idempotency: second run should detect existing data and skip ---
    munibot::db::migration::migrate_from_surrealdb(&db.pool, &surreal)
        .await
        .expect("second migration run should succeed (no-op)");

    // all counts must be unchanged -- no duplicates inserted
    let mut conn = db.pool.get().await.unwrap();

    let config_count: i64 = guild_configs::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let timer_count: i64 = autodelete_timers::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let wallet_count: i64 = guild_wallets::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let payout_count: i64 = guild_payouts::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let community_count: i64 = community_links::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let quote_count: i64 = quotes::table.count().get_result(&mut conn).await.unwrap();

    assert_eq!(
        config_count, 2,
        "idempotency: guild_configs count unchanged"
    );
    assert_eq!(
        timer_count, 3,
        "idempotency: autodelete_timers count unchanged"
    );
    assert_eq!(
        wallet_count, 2,
        "idempotency: guild_wallets count unchanged"
    );
    assert_eq!(
        payout_count, 2,
        "idempotency: guild_payouts count unchanged"
    );
    assert_eq!(
        community_count, 1,
        "idempotency: community_links count unchanged"
    );
    assert_eq!(quote_count, 3, "idempotency: quotes count unchanged");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_migration_with_empty_source() {
    let _ = env_logger::try_init();
    let db = TestDb::new().await;
    let surreal = connect_surreal().await;

    // no seeding -- surreal is empty, migration should produce zero rows
    munibot::db::migration::migrate_from_surrealdb(&db.pool, &surreal)
        .await
        .expect("migration of empty source should succeed");

    let mut conn = db.pool.get().await.unwrap();

    let config_count: i64 = guild_configs::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let timer_count: i64 = autodelete_timers::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let wallet_count: i64 = guild_wallets::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let payout_count: i64 = guild_payouts::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let community_count: i64 = community_links::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let quote_count: i64 = quotes::table.count().get_result(&mut conn).await.unwrap();

    assert_eq!(
        config_count, 0,
        "empty source: guild_configs should be empty"
    );
    assert_eq!(
        timer_count, 0,
        "empty source: autodelete_timers should be empty"
    );
    assert_eq!(
        wallet_count, 0,
        "empty source: guild_wallets should be empty"
    );
    assert_eq!(
        payout_count, 0,
        "empty source: guild_payouts should be empty"
    );
    assert_eq!(quote_count, 0, "empty source: quotes should be empty");

    // the default community link is always created, even when there are no
    // source quotes to migrate -- verify its presence and content
    assert_eq!(
        community_count, 1,
        "empty source: default community_link should still be created"
    );
    let community_rows: Vec<CommunityLink> = community_links::table.load(&mut conn).await.unwrap();
    assert_eq!(
        community_rows[0].twitch_streamer_id.as_deref(),
        Some("590712444"),
    );
    assert_eq!(community_rows[0].discord_guild_id, None);
    drop(conn);

    // second run should be a no-op: the idempotency check also inspects
    // community_links count, so the existing default row prevents a re-run
    // (and a unique constraint violation on the community link insert)
    munibot::db::migration::migrate_from_surrealdb(&db.pool, &surreal)
        .await
        .expect("second migration run on empty source should succeed");

    let mut conn = db.pool.get().await.unwrap();
    let config_count: i64 = guild_configs::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let community_count: i64 = community_links::table
        .count()
        .get_result(&mut conn)
        .await
        .unwrap();
    let quote_count: i64 = quotes::table.count().get_result(&mut conn).await.unwrap();
    assert_eq!(config_count, 0, "still empty after second run");
    assert_eq!(
        community_count, 1,
        "community_link unchanged after second run"
    );
    assert_eq!(quote_count, 0, "still empty after second run");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_migration_skips_non_numeric_logging_channel_id() {
    let _ = env_logger::try_init();
    let db = TestDb::new().await;
    let surreal = connect_surreal().await;

    // one valid numeric-keyed logging channel and one with a string key --
    // the string-keyed one should be skipped with a warning, not cause a failure
    surreal
        .query("CREATE logging_channel:111 SET channel_id = 222")
        .await
        .expect("seed valid logging_channel");
    surreal
        .query("CREATE logging_channel:bad_key SET channel_id = 333")
        .await
        .expect("seed invalid logging_channel");

    munibot::db::migration::migrate_from_surrealdb(&db.pool, &surreal)
        .await
        .expect("migration should succeed even with a non-numeric logging channel id");

    let mut conn = db.pool.get().await.unwrap();
    let configs: Vec<GuildConfig> = guild_configs::table
        .order(guild_configs::guild_id.asc())
        .load(&mut conn)
        .await
        .unwrap();

    assert_eq!(
        configs.len(),
        1,
        "only the numeric-keyed logging channel should be migrated"
    );
    assert_eq!(configs[0].guild_id, 111);
    assert_eq!(configs[0].logging_channel, Some(222));
}
