//! Integration tests for `db::operations::ai::pipeline`.
//!
//! Each test gets its own isolated MySQL database via `TestDb`. MySQL must be
//! running with the devenv credentials before running these tests.

mod common;

use common::TestDb;
use munibot_core::db::operations::ai;

#[tokio::test]
async fn test_create_pipeline_returns_a_row_with_the_given_identity() {
    let db = TestDb::new().await;
    let created = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 42)
        .await
        .expect("create failed");

    assert_eq!(created.forge, "github");
    assert_eq!(created.owner, "musicaloft");
    assert_eq!(created.repo_name, "munibot");
    assert_eq!(created.issue_number, 42);
}

#[tokio::test]
async fn test_get_pipeline_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_pipeline(&db.pool, 999999)
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_get_pipeline_finds_a_created_row() {
    let db = TestDb::new().await;
    let created = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");

    let found = ai::get_pipeline(&db.pool, created.id)
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(found.id, created.id);
}

#[tokio::test]
async fn test_list_pipeline_events_is_empty_for_a_pipeline_with_none() {
    let db = TestDb::new().await;
    let pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");

    let events = ai::list_pipeline_events(&db.pool, pipeline.id)
        .await
        .expect("query failed");
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_append_pipeline_event_assigns_sequential_sequence_numbers() {
    let db = TestDb::new().await;
    let pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");

    let first = ai::append_pipeline_event(&db.pool, pipeline.id, "Triggered", "{}")
        .await
        .expect("append failed");
    let second = ai::append_pipeline_event(&db.pool, pipeline.id, "ResearchCompleted", "{}")
        .await
        .expect("append failed");

    assert_eq!(first.seq, 0);
    assert_eq!(second.seq, 1);
}

#[tokio::test]
async fn test_list_pipeline_events_returns_them_in_append_order() {
    let db = TestDb::new().await;
    let pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");

    ai::append_pipeline_event(&db.pool, pipeline.id, "Triggered", "{}")
        .await
        .expect("append failed");
    ai::append_pipeline_event(&db.pool, pipeline.id, "ResearchCompleted", "{}")
        .await
        .expect("append failed");
    ai::append_pipeline_event(&db.pool, pipeline.id, "PlanCreated", "{}")
        .await
        .expect("append failed");

    let events = ai::list_pipeline_events(&db.pool, pipeline.id)
        .await
        .expect("query failed");

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type, "Triggered");
    assert_eq!(events[1].event_type, "ResearchCompleted");
    assert_eq!(events[2].event_type, "PlanCreated");
}

#[tokio::test]
async fn test_events_for_two_different_pipelines_are_independent() {
    let db = TestDb::new().await;
    let first_pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");
    let second_pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 2)
        .await
        .expect("create failed");

    ai::append_pipeline_event(&db.pool, first_pipeline.id, "Triggered", "{}")
        .await
        .expect("append failed");

    let first_events = ai::list_pipeline_events(&db.pool, first_pipeline.id)
        .await
        .expect("query failed");
    let second_events = ai::list_pipeline_events(&db.pool, second_pipeline.id)
        .await
        .expect("query failed");

    assert_eq!(first_events.len(), 1);
    assert!(second_events.is_empty());
}

#[tokio::test]
async fn test_deleting_a_pipeline_cascades_to_its_events() {
    let db = TestDb::new().await;
    let pipeline = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");
    ai::append_pipeline_event(&db.pool, pipeline.id, "Triggered", "{}")
        .await
        .expect("append failed");

    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use munibot_core::db::schema::ai_pipelines;

    let mut conn = db.pool.get().await.expect("couldn't get db connection");
    diesel::delete(ai_pipelines::table.find(pipeline.id))
        .execute(&mut conn)
        .await
        .expect("delete failed");
    drop(conn);

    let events = ai::list_pipeline_events(&db.pool, pipeline.id)
        .await
        .expect("query failed");
    assert!(
        events.is_empty(),
        "cascading delete should remove the events too"
    );
}

#[tokio::test]
async fn test_list_pipeline_ids_lists_every_created_pipeline() {
    let db = TestDb::new().await;
    let first = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");
    let second = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 2)
        .await
        .expect("create failed");

    let mut ids = ai::list_pipeline_ids(&db.pool).await.expect("query failed");
    ids.sort();
    assert_eq!(ids, vec![first.id, second.id]);
}

#[tokio::test]
async fn test_list_pipeline_ids_is_empty_for_a_fresh_database() {
    let db = TestDb::new().await;
    assert!(
        ai::list_pipeline_ids(&db.pool)
            .await
            .expect("query failed")
            .is_empty()
    );
}

#[tokio::test]
async fn test_list_pipelines_returns_full_rows_most_recently_created_first() {
    let db = TestDb::new().await;
    let first = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 1)
        .await
        .expect("create failed");
    let second = ai::create_pipeline(&db.pool, "github", "musicaloft", "munibot", 2)
        .await
        .expect("create failed");

    let pipelines = ai::list_pipelines(&db.pool).await.expect("query failed");
    assert_eq!(pipelines.len(), 2);
    assert_eq!(pipelines[0].id, second.id);
    assert_eq!(pipelines[1].id, first.id);
}
