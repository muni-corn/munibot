//! Integration tests for [`DieselProjectStore`] against a real database.
//!
//! Skipped entirely unless `MUNIBOT_AI_TEST_DATABASE_URL` is set, matching
//! `diesel_pipeline_store.rs` - see that file's module doc for the full
//! rationale. Tests share the database and isolate themselves with a
//! unique slug per test rather than creating a database each.

use std::sync::atomic::{AtomicU64, Ordering};

use munibot_ai::{
    project::{
        DieselProjectStore, ImageStatus, ImageStrategy, NewProject, NewWorkspace, ProjectStore,
    },
    sandbox::config::SandboxNetworkConfig,
};
use munibot_core::db::{DbPool, establish_pool};
use munibot_vcs::{Forge, RepoRef};

async fn pool() -> Option<DbPool> {
    let url = std::env::var("MUNIBOT_AI_TEST_DATABASE_URL").ok()?;
    unsafe { std::env::set_var("DATABASE_URL", url) };
    Some(establish_pool().await.expect("couldn't build a pool"))
}

fn unique_spec() -> NewProject {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let n = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    NewProject {
        slug: format!("cocoa-{n}"),
        display_name: "Cocoa".to_string(),
        repo: RepoRef::new(Forge::GitHub, "muni-corn", "cocoa"),
        clone_url: "https://github.com/muni-corn/cocoa.git".to_string(),
        default_branch: "main".to_string(),
        local_path: std::path::PathBuf::from(format!("/var/lib/munibot/projects/1/cocoa-{n}")),
        image_strategy: ImageStrategy::Base,
        network_policy: SandboxNetworkConfig::None,
        trigger: None,
        installation_id: None,
    }
}

#[tokio::test]
async fn test_upsert_project_and_get_project_round_trip() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = unique_spec();

    let created = store.upsert_project(&spec).await.expect("upsert failed");
    let found = store.get_project(created.id).await.expect("get failed");

    assert_eq!(found.slug, spec.slug);
    assert_eq!(found.image_status, ImageStatus::Unbuilt);
}

#[tokio::test]
async fn test_upsert_project_round_trips_an_installation_id() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = NewProject {
        installation_id: Some(4242),
        ..unique_spec()
    };

    let created = store.upsert_project(&spec).await.expect("upsert failed");
    assert_eq!(created.installation_id, Some(4242));

    let found = store.get_project(created.id).await.expect("get failed");
    assert_eq!(found.installation_id, Some(4242));
}

#[tokio::test]
async fn test_a_manually_registered_project_has_no_installation_id() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = unique_spec();
    assert_eq!(spec.installation_id, None);

    let created = store.upsert_project(&spec).await.expect("upsert failed");
    assert_eq!(created.installation_id, None);
}

#[tokio::test]
async fn test_upsert_project_on_a_slug_collision_updates_the_same_row() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let mut spec = unique_spec();

    let first = store
        .upsert_project(&spec)
        .await
        .expect("first upsert failed");
    spec.display_name = "Cocoa (renamed)".to_string();
    let updated = store
        .upsert_project(&spec)
        .await
        .expect("second upsert failed");

    assert_eq!(updated.id, first.id);
    assert_eq!(updated.display_name, "Cocoa (renamed)");
}

#[tokio::test]
async fn test_update_image_status_persists_and_reads_back_a_built_status() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = unique_spec();
    let project = store.upsert_project(&spec).await.expect("upsert failed");

    let now = chrono::Utc::now();
    // MySQL's DATETIME has second precision - truncate so the round trip
    // compares equal rather than off by sub-second fractions
    let now = chrono::DateTime::from_timestamp(now.timestamp(), 0).unwrap();

    store
        .update_image_status(project.id, ImageStrategy::Devenv, ImageStatus::Built {
            tag: "munibot-project-cocoa:abc123".to_string(),
            hash: "abc123".to_string(),
            at: now,
        })
        .await
        .expect("update failed");

    let found = store.get_project(project.id).await.expect("get failed");
    assert_eq!(found.image_strategy, ImageStrategy::Devenv);
    assert_eq!(found.image_status, ImageStatus::Built {
        tag: "munibot-project-cocoa:abc123".to_string(),
        hash: "abc123".to_string(),
        at: now,
    });
}

#[tokio::test]
async fn test_create_workspace_and_list_workspaces_for_project() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = unique_spec();
    let project = store.upsert_project(&spec).await.expect("upsert failed");

    store
        .create_workspace(&NewWorkspace {
            project_id: project.id,
            name: "pipeline-142".to_string(),
            branch: "munibot/issue-17".to_string(),
            path: project.local_path.join("worktrees/pipeline-142"),
            pipeline_id: Some(142),
            conversation_id: None,
        })
        .await
        .expect("create workspace failed");

    let workspaces = store
        .list_workspaces_for_project(project.id)
        .await
        .expect("list failed");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].name, "pipeline-142");
    assert_eq!(workspaces[0].pipeline_id, Some(142));
}

#[tokio::test]
async fn test_delete_project_cascades_to_its_workspaces() {
    let Some(pool) = pool().await else { return };
    let store = DieselProjectStore::new(pool);
    let spec = unique_spec();
    let project = store.upsert_project(&spec).await.expect("upsert failed");
    let workspace = store
        .create_workspace(&NewWorkspace {
            project_id: project.id,
            name: "scratch".to_string(),
            branch: "munibot/scratch".to_string(),
            path: project.local_path.join("worktrees/scratch"),
            pipeline_id: None,
            conversation_id: None,
        })
        .await
        .expect("create workspace failed");

    store
        .delete_project(project.id)
        .await
        .expect("delete failed");

    let error = store
        .get_workspace(workspace.id)
        .await
        .expect_err("the workspace should have been cascade-deleted");
    assert!(!error.to_string().is_empty());
}
