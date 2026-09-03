//! Integration tests for `db::operations::ai::project`.
//!
//! Each test gets its own isolated MySQL database via `TestDb`. MySQL must be
//! running with the devenv credentials before running these tests.

mod common;

use chrono::Utc;
use common::TestDb;
use munibot_core::db::{
    models::{NewAiProject, NewAiWorkspace, UpdateAiProject},
    operations::ai,
};

fn sample_project(slug: &str) -> NewAiProject {
    let now = Utc::now().naive_utc();
    NewAiProject {
        slug: slug.to_string(),
        display_name: "Cocoa".to_string(),
        forge: "github".to_string(),
        repo_owner: "muni-corn".to_string(),
        repo_name: "cocoa".to_string(),
        clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
        default_branch: "main".to_string(),
        local_path: "/var/lib/munibot/projects/1/cocoa".to_string(),
        image_strategy: "base".to_string(),
        image_tag: None,
        image_source_hash: None,
        image_built_at: None,
        image_error: None,
        network_policy: "none".to_string(),
        trigger_mode: None,
        trigger_label: None,
        created_at: now,
        updated_at: now,
        installation_id: None,
    }
}

#[tokio::test]
async fn test_upsert_project_creates_a_new_row() {
    let db = TestDb::new().await;
    let created = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    assert_eq!(created.slug, "cocoa");
    assert_eq!(created.repo_owner, "muni-corn");
    assert_eq!(created.image_strategy, "base");
}

#[tokio::test]
async fn test_upsert_project_on_a_slug_collision_updates_instead_of_erroring() {
    let db = TestDb::new().await;
    let first = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("first upsert failed");

    let mut second = sample_project("cocoa");
    second.display_name = "Cocoa (renamed)".to_string();
    let updated = ai::upsert_project(&db.pool, &second)
        .await
        .expect("second upsert failed");

    assert_eq!(
        updated.id, first.id,
        "should update the same row, not insert a second one"
    );
    assert_eq!(updated.display_name, "Cocoa (renamed)");

    let all = ai::list_projects(&db.pool).await.expect("query failed");
    assert_eq!(
        all.len(),
        1,
        "a slug collision must not create a duplicate row"
    );
}

#[tokio::test]
async fn test_get_project_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_project(&db.pool, 999999)
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_get_project_by_slug_finds_a_created_row() {
    let db = TestDb::new().await;
    ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    let found = ai::get_project_by_slug(&db.pool, "cocoa")
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(found.slug, "cocoa");
}

#[tokio::test]
async fn test_get_project_by_slug_missing_returns_none() {
    let db = TestDb::new().await;
    assert!(
        ai::get_project_by_slug(&db.pool, "does-not-exist")
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_list_projects_lists_every_created_project() {
    let db = TestDb::new().await;
    ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    ai::upsert_project(&db.pool, &sample_project("munibot"))
        .await
        .expect("upsert failed");

    let mut slugs: Vec<_> = ai::list_projects(&db.pool)
        .await
        .expect("query failed")
        .into_iter()
        .map(|p| p.slug)
        .collect();
    slugs.sort();
    assert_eq!(slugs, vec!["cocoa".to_string(), "munibot".to_string()]);
}

#[tokio::test]
async fn test_update_project_applies_only_the_given_fields() {
    let db = TestDb::new().await;
    let created = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    let changes = UpdateAiProject {
        display_name: None,
        default_branch: None,
        image_strategy: Some("devenv".to_string()),
        image_tag: Some(Some("munibot-project-cocoa:abc123".to_string())),
        image_source_hash: Some(Some("abc123".to_string())),
        image_built_at: Some(Some(Utc::now().naive_utc())),
        image_error: None,
        network_policy: None,
        trigger_mode: None,
        trigger_label: None,
        updated_at: Utc::now().naive_utc(),
    };
    let updated = ai::update_project(&db.pool, created.id, &changes)
        .await
        .expect("update failed");

    assert_eq!(updated.image_strategy, "devenv");
    assert_eq!(
        updated.image_tag,
        Some("munibot-project-cocoa:abc123".to_string())
    );
    // untouched fields survive the partial update
    assert_eq!(updated.display_name, "Cocoa");
    assert_eq!(updated.repo_owner, "muni-corn");
}

#[tokio::test]
async fn test_delete_project_removes_the_row() {
    let db = TestDb::new().await;
    let created = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    ai::delete_project(&db.pool, created.id)
        .await
        .expect("delete failed");

    assert!(
        ai::get_project(&db.pool, created.id)
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_deleting_a_project_cascades_to_its_workspaces() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    let now = Utc::now().naive_utc();
    let workspace = ai::create_workspace(&db.pool, &NewAiWorkspace {
        project_id: project.id,
        name: "pipeline-142".to_string(),
        branch: "munibot/issue-17".to_string(),
        path: "/var/lib/munibot/projects/1/cocoa/worktrees/pipeline-142".to_string(),
        pipeline_id: Some(142),
        conversation_id: None,
        created_at: now,
        last_used_at: now,
    })
    .await
    .expect("create workspace failed");

    ai::delete_project(&db.pool, project.id)
        .await
        .expect("delete failed");

    assert!(
        ai::get_workspace(&db.pool, workspace.id)
            .await
            .expect("query failed")
            .is_none(),
        "cascading delete should remove the workspace too"
    );
}

fn sample_workspace(project_id: i64, name: &str) -> NewAiWorkspace {
    let now = Utc::now().naive_utc();
    NewAiWorkspace {
        project_id,
        name: name.to_string(),
        branch: format!("munibot/{name}"),
        path: format!("/var/lib/munibot/projects/1/cocoa/worktrees/{name}"),
        pipeline_id: None,
        conversation_id: None,
        created_at: now,
        last_used_at: now,
    }
}

#[tokio::test]
async fn test_create_workspace_returns_a_row_with_the_given_identity() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    let created = ai::create_workspace(&db.pool, &sample_workspace(project.id, "pipeline-142"))
        .await
        .expect("create failed");

    assert_eq!(created.project_id, project.id);
    assert_eq!(created.name, "pipeline-142");
}

#[tokio::test]
async fn test_create_workspace_rejects_a_duplicate_name_within_the_same_project() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    ai::create_workspace(&db.pool, &sample_workspace(project.id, "scratch"))
        .await
        .expect("first create failed");

    let error = ai::create_workspace(&db.pool, &sample_workspace(project.id, "scratch"))
        .await
        .expect_err("a duplicate (project_id, name) should be rejected");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn test_get_workspace_by_name_finds_a_created_row() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    ai::create_workspace(&db.pool, &sample_workspace(project.id, "pipeline-142"))
        .await
        .expect("create failed");

    let found = ai::get_workspace_by_name(&db.pool, project.id, "pipeline-142")
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(found.name, "pipeline-142");
}

#[tokio::test]
async fn test_get_workspace_by_name_missing_returns_none() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");

    assert!(
        ai::get_workspace_by_name(&db.pool, project.id, "does-not-exist")
            .await
            .expect("query failed")
            .is_none()
    );
}

#[tokio::test]
async fn test_list_workspaces_for_project_only_lists_that_projects_workspaces() {
    let db = TestDb::new().await;
    let cocoa = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    let munibot = ai::upsert_project(&db.pool, &sample_project("munibot"))
        .await
        .expect("upsert failed");

    ai::create_workspace(&db.pool, &sample_workspace(cocoa.id, "scratch-a"))
        .await
        .expect("create failed");
    ai::create_workspace(&db.pool, &sample_workspace(cocoa.id, "scratch-b"))
        .await
        .expect("create failed");
    ai::create_workspace(&db.pool, &sample_workspace(munibot.id, "scratch-c"))
        .await
        .expect("create failed");

    let cocoa_workspaces = ai::list_workspaces_for_project(&db.pool, cocoa.id)
        .await
        .expect("query failed");
    assert_eq!(cocoa_workspaces.len(), 2);
}

#[tokio::test]
async fn test_touch_workspace_updates_last_used_at() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    let created = ai::create_workspace(&db.pool, &sample_workspace(project.id, "scratch"))
        .await
        .expect("create failed");

    let later = created.last_used_at + chrono::Duration::hours(1);
    ai::touch_workspace(&db.pool, created.id, later)
        .await
        .expect("touch failed");

    let found = ai::get_workspace(&db.pool, created.id)
        .await
        .expect("query failed")
        .expect("should exist");
    assert_eq!(found.last_used_at, later);
}

#[tokio::test]
async fn test_delete_workspace_removes_the_row() {
    let db = TestDb::new().await;
    let project = ai::upsert_project(&db.pool, &sample_project("cocoa"))
        .await
        .expect("upsert failed");
    let created = ai::create_workspace(&db.pool, &sample_workspace(project.id, "scratch"))
        .await
        .expect("create failed");

    ai::delete_workspace(&db.pool, created.id)
        .await
        .expect("delete failed");

    assert!(
        ai::get_workspace(&db.pool, created.id)
            .await
            .expect("query failed")
            .is_none()
    );
}
