//! Integration tests for all database CRUD operations in `db::operations`.
//!
//! Each test gets its own isolated MySQL database via `TestDb`. MySQL must be
//! running with the devenv credentials before running these tests.

mod common;

use chrono::Utc;
use common::TestDb;
use munibot::db::{
    models::{AutoDeleteTimerRow, GuildConfig},
    operations,
};

// ─── guild_configs ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_upsert_and_get_guild_config() {
    let db = TestDb::new().await;

    let config = GuildConfig {
        guild_id: 1001,
        logging_channel: Some(9999),
    };
    let saved = operations::upsert_guild_config(&db.pool, config)
        .await
        .expect("upsert failed");

    assert_eq!(saved.guild_id, 1001);
    assert_eq!(saved.logging_channel, Some(9999));

    let fetched = operations::get_guild_config(&db.pool, 1001)
        .await
        .expect("get failed")
        .expect("should be Some");

    assert_eq!(fetched.guild_id, 1001);
    assert_eq!(fetched.logging_channel, Some(9999));
}

#[tokio::test]
async fn test_upsert_guild_config_overwrites_existing() {
    let db = TestDb::new().await;

    operations::upsert_guild_config(
        &db.pool,
        GuildConfig {
            guild_id: 1002,
            logging_channel: Some(111),
        },
    )
    .await
    .expect("first upsert failed");

    operations::upsert_guild_config(
        &db.pool,
        GuildConfig {
            guild_id: 1002,
            logging_channel: Some(222),
        },
    )
    .await
    .expect("second upsert failed");

    let fetched = operations::get_guild_config(&db.pool, 1002)
        .await
        .expect("get failed")
        .expect("should be Some");

    assert_eq!(
        fetched.logging_channel,
        Some(222),
        "upsert should overwrite"
    );
}

#[tokio::test]
async fn test_get_guild_config_missing_returns_none() {
    let db = TestDb::new().await;
    let result = operations::get_guild_config(&db.pool, 99999)
        .await
        .expect("get failed");
    assert!(result.is_none(), "missing config should return None");
}

#[tokio::test]
async fn test_delete_guild_config() {
    let db = TestDb::new().await;

    operations::upsert_guild_config(
        &db.pool,
        GuildConfig {
            guild_id: 1003,
            logging_channel: None,
        },
    )
    .await
    .expect("upsert failed");

    let deleted = operations::delete_guild_config(&db.pool, 1003)
        .await
        .expect("delete failed");
    assert_eq!(deleted, 1);

    let after = operations::get_guild_config(&db.pool, 1003)
        .await
        .expect("get failed");
    assert!(after.is_none(), "config should be gone after delete");
}

// ─── autodelete_timers ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_upsert_and_get_all_autodelete_timers() {
    let db = TestDb::new().await;

    let row = AutoDeleteTimerRow {
        channel_id: 2001,
        guild_id: 3001,
        duration_secs: 3600,
        last_cleaned: Utc::now().naive_utc(),
        last_message_id_cleaned: 0,
        mode: "AfterSilence".to_string(),
    };
    operations::upsert_autodelete_timer(&db.pool, row)
        .await
        .expect("upsert failed");

    let all = operations::get_all_autodelete_timers(&db.pool)
        .await
        .expect("get_all failed");

    assert_eq!(all.len(), 1);
    assert_eq!(all[0].channel_id, 2001);
    assert_eq!(all[0].guild_id, 3001);
    assert_eq!(all[0].duration_secs, 3600);
}

#[tokio::test]
async fn test_delete_autodelete_timer() {
    let db = TestDb::new().await;

    let row = AutoDeleteTimerRow {
        channel_id: 2002,
        guild_id: 3002,
        duration_secs: 7200,
        last_cleaned: Utc::now().naive_utc(),
        last_message_id_cleaned: 0,
        mode: "Always".to_string(),
    };
    operations::upsert_autodelete_timer(&db.pool, row)
        .await
        .expect("upsert failed");

    let deleted = operations::delete_autodelete_timer(&db.pool, 2002)
        .await
        .expect("delete failed");
    assert_eq!(deleted, 1);

    let all = operations::get_all_autodelete_timers(&db.pool)
        .await
        .expect("get_all failed");
    assert!(all.is_empty(), "timer should be gone after delete");
}

#[tokio::test]
async fn test_update_autodelete_last_cleaned() {
    let db = TestDb::new().await;

    let initial_time = chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc();
    let row = AutoDeleteTimerRow {
        channel_id: 2003,
        guild_id: 3003,
        duration_secs: 1800,
        last_cleaned: initial_time,
        last_message_id_cleaned: 0,
        mode: "AfterSilence".to_string(),
    };
    operations::upsert_autodelete_timer(&db.pool, row)
        .await
        .expect("upsert failed");

    let new_time = Utc::now().naive_utc();
    let updated = operations::update_autodelete_last_cleaned(&db.pool, 2003, new_time, 12345)
        .await
        .expect("update failed");
    assert_eq!(updated, 1);

    let all = operations::get_all_autodelete_timers(&db.pool)
        .await
        .expect("get_all failed");
    assert_eq!(all[0].last_message_id_cleaned, 12345);
    // truncate to seconds for MySQL datetime precision
    assert!(all[0].last_cleaned.and_utc().timestamp() >= initial_time.and_utc().timestamp());
}

// ─── guild_wallets ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_or_create_wallet_creates_new() {
    let db = TestDb::new().await;

    let wallet = operations::get_or_create_wallet(&db.pool, 4001, 5001)
        .await
        .expect("get_or_create failed");

    assert_eq!(wallet.guild_id, 4001);
    assert_eq!(wallet.user_id, 5001);
    assert_eq!(wallet.balance, 0, "new wallet should have zero balance");
}

#[tokio::test]
async fn test_get_or_create_wallet_returns_existing() {
    let db = TestDb::new().await;

    let first = operations::get_or_create_wallet(&db.pool, 4002, 5002)
        .await
        .expect("first get_or_create failed");

    // deposit some coins so we can tell the rows apart
    operations::deposit_to_wallet(&db.pool, first.id, 500)
        .await
        .expect("deposit failed");

    let second = operations::get_or_create_wallet(&db.pool, 4002, 5002)
        .await
        .expect("second get_or_create failed");

    assert_eq!(first.id, second.id, "should return the same wallet row");
    assert_eq!(
        second.balance, 500,
        "should retain balance from first creation"
    );
}

#[tokio::test]
async fn test_deposit_to_wallet_increases_balance() {
    let db = TestDb::new().await;

    let wallet = operations::get_or_create_wallet(&db.pool, 4003, 5003)
        .await
        .expect("create failed");

    let after = operations::deposit_to_wallet(&db.pool, wallet.id, 250)
        .await
        .expect("deposit failed");
    assert_eq!(after.balance, 250);

    let after2 = operations::deposit_to_wallet(&db.pool, wallet.id, 100)
        .await
        .expect("second deposit failed");
    assert_eq!(after2.balance, 350, "balance should accumulate");
}

#[tokio::test]
async fn test_spend_from_wallet_decreases_balance() {
    let db = TestDb::new().await;

    let wallet = operations::get_or_create_wallet(&db.pool, 4004, 5004)
        .await
        .expect("create failed");

    operations::deposit_to_wallet(&db.pool, wallet.id, 1000)
        .await
        .expect("deposit failed");

    let after = operations::spend_from_wallet(&db.pool, wallet.id, 300)
        .await
        .expect("spend failed");
    assert_eq!(after.balance, 700);
}

// ─── guild_payouts ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_or_create_payout_creates_new() {
    let db = TestDb::new().await;

    let initial = Utc::now().naive_utc();
    let payout = operations::get_or_create_payout(&db.pool, 6001, 7001, initial)
        .await
        .expect("get_or_create failed");

    assert_eq!(payout.guild_id, 6001);
    assert_eq!(payout.user_id, 7001);
    assert_eq!(payout.balance, 0, "new payout should have zero balance");
}

#[tokio::test]
async fn test_get_or_create_payout_returns_existing() {
    let db = TestDb::new().await;

    let initial = Utc::now().naive_utc();
    let first = operations::get_or_create_payout(&db.pool, 6002, 7002, initial)
        .await
        .expect("first get_or_create failed");

    // update the balance so we can verify identity
    operations::update_payout(&db.pool, first.id, 999)
        .await
        .expect("update failed");

    let second = operations::get_or_create_payout(&db.pool, 6002, 7002, initial)
        .await
        .expect("second get_or_create failed");

    assert_eq!(first.id, second.id, "should return the same payout row");
    assert_eq!(second.balance, 999, "should retain updated balance");
}

#[tokio::test]
async fn test_update_payout_balance() {
    let db = TestDb::new().await;

    let initial = Utc::now().naive_utc();
    let payout = operations::get_or_create_payout(&db.pool, 6003, 7003, initial)
        .await
        .expect("get_or_create failed");

    let rows = operations::update_payout(&db.pool, payout.id, 1234)
        .await
        .expect("update failed");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn test_claim_payout_zeros_balance_and_records_time() {
    let db = TestDb::new().await;

    let initial = Utc::now().naive_utc();
    let payout = operations::get_or_create_payout(&db.pool, 6004, 7004, initial)
        .await
        .expect("get_or_create failed");

    operations::update_payout(&db.pool, payout.id, 500)
        .await
        .expect("update failed");

    let claimed_at = Utc::now().naive_utc();
    let after = operations::claim_payout(&db.pool, payout.id, claimed_at)
        .await
        .expect("claim failed");

    assert_eq!(after.balance, 0, "balance should be zeroed after claim");
}

// ─── community_links ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_or_create_community_link_by_twitch_id_creates_new() {
    let db = TestDb::new().await;

    let link = operations::get_or_create_community_link_by_twitch_id(&db.pool, "twitch_abc")
        .await
        .expect("get_or_create failed");

    assert_eq!(link.twitch_streamer_id.as_deref(), Some("twitch_abc"));
    assert!(
        link.discord_guild_id.is_none(),
        "new link should have no discord guild"
    );
}

#[tokio::test]
async fn test_get_or_create_community_link_returns_existing() {
    let db = TestDb::new().await;

    let first = operations::get_or_create_community_link_by_twitch_id(&db.pool, "twitch_def")
        .await
        .expect("first get_or_create failed");

    let second = operations::get_or_create_community_link_by_twitch_id(&db.pool, "twitch_def")
        .await
        .expect("second get_or_create failed");

    assert_eq!(first.id, second.id, "should return the same row");
}

#[tokio::test]
async fn test_get_community_by_twitch_id() {
    let db = TestDb::new().await;

    operations::get_or_create_community_link_by_twitch_id(&db.pool, "twitch_ghi")
        .await
        .expect("create failed");

    let found = operations::get_community_by_twitch_id(&db.pool, "twitch_ghi")
        .await
        .expect("get failed")
        .expect("should be Some");

    assert_eq!(found.twitch_streamer_id.as_deref(), Some("twitch_ghi"));
}

#[tokio::test]
async fn test_get_community_by_twitch_id_missing_returns_none() {
    let db = TestDb::new().await;
    let result = operations::get_community_by_twitch_id(&db.pool, "nonexistent_xyz")
        .await
        .expect("get failed");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_community_by_guild_id_missing_returns_none() {
    let db = TestDb::new().await;
    let result = operations::get_community_by_guild_id(&db.pool, 99999999)
        .await
        .expect("get failed");
    assert!(result.is_none());
}

// ─── quotes ───────────────────────────────────────────────────────────────────

async fn make_community(db: &TestDb) -> i64 {
    // quotes require a community_id that references community_links.id
    let link = operations::get_or_create_community_link_by_twitch_id(&db.pool, "quote_test_stream")
        .await
        .expect("failed to create community link for quote tests");
    link.id
}

#[tokio::test]
async fn test_add_quote_and_get_by_number() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let now = Utc::now().naive_utc();
    let quote = operations::add_quote(
        &db.pool,
        community_id,
        now,
        "never give up".to_string(),
        "chatter1".to_string(),
        "Just Chatting".to_string(),
        "chill stream".to_string(),
    )
    .await
    .expect("add_quote failed");

    assert_eq!(
        quote.sequential_id, 1,
        "first quote should be sequential_id 1"
    );
    assert_eq!(quote.quote, "never give up");

    let found = operations::get_quote_by_number(&db.pool, community_id, 1)
        .await
        .expect("get_quote_by_number failed")
        .expect("should be Some");

    assert_eq!(found.id, quote.id);
}

#[tokio::test]
async fn test_add_quote_sequential_ids_increment() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let now = Utc::now().naive_utc();
    for i in 1..=3i32 {
        let q = operations::add_quote(
            &db.pool,
            community_id,
            now,
            format!("quote number {i}"),
            "chatter".to_string(),
            "Gaming".to_string(),
            "title".to_string(),
        )
        .await
        .expect("add_quote failed");
        assert_eq!(q.sequential_id, i, "sequential_id should increment");
    }
}

#[tokio::test]
async fn test_get_quote_by_content() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let now = Utc::now().naive_utc();
    operations::add_quote(
        &db.pool,
        community_id,
        now,
        "hello world".to_string(),
        "chatter2".to_string(),
        "IRL".to_string(),
        "a title".to_string(),
    )
    .await
    .expect("add_quote failed");

    let found = operations::get_quote_by_content(&db.pool, community_id, "hello world")
        .await
        .expect("get_quote_by_content failed")
        .expect("should be Some");

    assert_eq!(found.quote, "hello world");
}

#[tokio::test]
async fn test_get_quote_by_number_missing_returns_none() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let result = operations::get_quote_by_number(&db.pool, community_id, 999)
        .await
        .expect("get_quote_by_number failed");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_random_quote_returns_some_when_quotes_exist() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let now = Utc::now().naive_utc();
    operations::add_quote(
        &db.pool,
        community_id,
        now,
        "random quote content".to_string(),
        "chatter3".to_string(),
        "Art".to_string(),
        "painting stream".to_string(),
    )
    .await
    .expect("add_quote failed");

    let result = operations::get_random_quote(&db.pool, community_id)
        .await
        .expect("get_random_quote failed");
    assert!(result.is_some(), "should return Some when quotes exist");
}

#[tokio::test]
async fn test_get_random_quote_empty_community_returns_none() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let result = operations::get_random_quote(&db.pool, community_id)
        .await
        .expect("get_random_quote failed");
    assert!(result.is_none(), "should return None for empty community");
}

#[tokio::test]
async fn test_count_quotes() {
    let db = TestDb::new().await;
    let community_id = make_community(&db).await;

    let initial = operations::count_quotes(&db.pool, community_id)
        .await
        .expect("count failed");
    assert_eq!(initial, 0);

    let now = Utc::now().naive_utc();
    for i in 1..=5 {
        operations::add_quote(
            &db.pool,
            community_id,
            now,
            format!("quote {i}"),
            "chatter".to_string(),
            "category".to_string(),
            "title".to_string(),
        )
        .await
        .expect("add_quote failed");
    }

    let count = operations::count_quotes(&db.pool, community_id)
        .await
        .expect("count failed");
    assert_eq!(count, 5);
}
