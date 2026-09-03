use dioxus::prelude::*;

// only the server actually constructs a ChatError or a WorkspaceSummary --
// the client's own stub never runs a server function's body, the same
// reasoning munibot_api::server_fns::pipeline already documents
#[cfg(feature = "server")]
use crate::{chat::ChatError, project::WorkspaceSummary};
use crate::{
    chat::ChatResult,
    project::{ProjectDetail, ProjectPickerEntry, ProjectSummary, RegisterProjectForm},
};

/// Builds a project's wire summary from the domain type, recomputing its
/// image-strategy skip reasons fresh against its own default worktree -
/// `munibot_ai::project::detect_image_strategy` is a pure function over
/// the filesystem, cheap enough to call on every page load rather than
/// ever persisting its result (see that function's own doc comment).
#[cfg(feature = "server")]
fn summarize(project: &munibot_ai::project::Project, workspace_count: usize) -> ProjectSummary {
    use munibot_ai::project::ImageStatus;

    let (image_tag, image_built_at_unix, image_error) = match &project.image_status {
        ImageStatus::Unbuilt => (None, None, None),
        ImageStatus::Built { tag, at, .. } => (Some(tag.clone()), Some(at.timestamp()), None),
        ImageStatus::Failed { reason } => (None, None, Some(reason.clone())),
    };

    let default_workspace = project.local_path.join("worktrees").join("default");
    let (_, skip_reasons) = munibot_ai::project::detect_image_strategy(&default_workspace);

    ProjectSummary {
        id: project.id.0,
        slug: project.slug.clone(),
        display_name: project.display_name.clone(),
        repo: project.repo.to_string(),
        image_strategy: project.image_strategy.to_string(),
        image_status: project.image_status.label().to_string(),
        image_tag,
        image_built_at_unix,
        image_error,
        network_policy: project.network_policy.to_string(),
        skip_reasons: skip_reasons.into_iter().map(|s| s.reason).collect(),
        trigger: project.trigger.as_ref().map(describe_trigger),
        workspace_count,
        created_at_unix: project.created_at.timestamp(),
        updated_at_unix: project.updated_at.timestamp(),
    }
}

/// A human-readable description of a trigger mode - `TriggerMode` itself
/// has no `Display` impl (its variants carry data a single format string
/// can't cover uniformly), so this is a small, hand-written prose
/// projection rather than a general one.
#[cfg(feature = "server")]
fn describe_trigger(trigger: &munibot_vcs::TriggerMode) -> String {
    match trigger {
        munibot_vcs::TriggerMode::AllIssues => "every issue".to_string(),
        munibot_vcs::TriggerMode::Label(label) => format!("labeled {label:?}"),
        munibot_vcs::TriggerMode::Keyword(keyword) => format!("keyword {keyword:?}"),
        munibot_vcs::TriggerMode::Any(modes) => {
            let parts: Vec<String> = modes.iter().map(describe_trigger).collect();
            format!("any of: {}", parts.join(", "))
        }
    }
}

/// Whether any pipeline ever started against `project_id` has not yet
/// reached a terminal state - what [`remove_project`] refuses to remove
/// out from under. Folds each candidate's own event log through
/// `advance`, the same replay `list_pipelines`/`get_pipeline_detail`
/// already do for the pipeline monitor page.
#[cfg(feature = "server")]
async fn has_a_running_pipeline(
    pool: &munibot_core::db::DbPool,
    project_id: i64,
) -> ChatResult<bool> {
    use munibot_core::db::operations::ai;

    for row in ai::list_pipelines_for_project(pool, project_id).await? {
        let event_rows = ai::list_pipeline_events(pool, row.id).await?;
        let state = event_rows
            .iter()
            .map(|event_row| {
                serde_json::from_str::<munibot_ai::pipeline::PipelineEvent>(&event_row.payload)
                    .map_err(|error| ChatError::from(anyhow::anyhow!(error.to_string())))
            })
            .try_fold(
                munibot_ai::pipeline::PipelineState::Triaging,
                |state, event| {
                    let event = event?;
                    munibot_ai::pipeline::advance(state, &event)
                        .map_err(|error| ChatError::from(anyhow::anyhow!(error.to_string())))
                },
            )?;

        if !state.is_terminal() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Every registered project, most recently updated first. Restricted to
/// operators, the same reasoning `list_pipelines`'s own doc comment
/// gives: which repositories munibot has cloned is not everyone's
/// business.
#[server(
    auth: crate::auth::server::AuthSession,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn list_projects() -> ChatResult<Vec<ProjectSummary>> {
    crate::auth::operator::require_operator(&auth).await?;

    let mut summaries = Vec::new();
    for project in manager.list_projects().await? {
        let workspace_count = manager.list_workspaces(project.id).await?.len();
        summaries.push(summarize(&project, workspace_count));
    }
    // most recently updated first
    summaries.sort_by_key(|summary| std::cmp::Reverse(summary.updated_at_unix));
    Ok(summaries)
}

/// One project's own summary plus every workspace under it.
#[server(
    auth: crate::auth::server::AuthSession,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn get_project(project_id: i64) -> ChatResult<ProjectDetail> {
    crate::auth::operator::require_operator(&auth).await?;

    let project = manager
        .get_project(munibot_ai::project::ProjectId(project_id))
        .await?;
    let workspaces = manager.list_workspaces(project.id).await?;
    let summary = summarize(&project, workspaces.len());
    let workspaces = workspaces
        .into_iter()
        .map(|workspace| WorkspaceSummary {
            id: workspace.id.0,
            name: workspace.name,
            branch: workspace.branch,
            pipeline_id: workspace.pipeline_id,
            conversation_id: workspace.conversation_id,
            last_used_at_unix: workspace.last_used_at.timestamp(),
        })
        .collect();

    Ok(ProjectDetail {
        summary,
        workspaces,
    })
}

/// Registers a new project: clones it, checks out a default workspace,
/// detects its image strategy, and builds an image if one applies.
///
/// The registering operator's own user id becomes the project's
/// `munibot_user_id` - see `RegisterProjectSpec`'s own doc comment on the
/// on-disk layout this determines. No forge token is threaded through
/// this form yet, so only a repository cloneable without one (a public
/// repository, or one already reachable via an ssh agent/credential
/// helper on the host) can be registered this way; a private repository
/// is registered automatically instead, through a GitHub App
/// installation (`ProjectManager::register_from_installation`).
#[server(
    auth: crate::auth::server::AuthSession,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn register_project(form: RegisterProjectForm) -> ChatResult<ProjectSummary> {
    crate::auth::operator::require_operator(&auth).await?;
    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;

    let project = manager
        .register(munibot_ai::project::RegisterProjectSpec {
            munibot_user_id: user.id,
            slug: form.slug,
            display_name: form.display_name,
            repo: munibot_vcs::RepoRef::new(
                munibot_vcs::Forge::GitHub,
                form.repo_owner,
                form.repo_name,
            ),
            clone_url: form.clone_url,
            default_branch: form.default_branch,
            network_policy: munibot_ai::sandbox::config::SandboxNetworkConfig::None,
            trigger: None,
            token: None,
            installation_id: None,
        })
        .await?;

    let workspace_count = manager.list_workspaces(project.id).await?.len();
    Ok(summarize(&project, workspace_count))
}

/// Removes a project entirely: every worktree, the bare clone, and the
/// row itself. Refuses while a pipeline is still running against it -
/// see [`has_a_running_pipeline`].
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn remove_project(project_id: i64) -> ChatResult<()> {
    crate::auth::operator::require_operator(&auth).await?;

    let project = manager
        .get_project(munibot_ai::project::ProjectId(project_id))
        .await?;

    if has_a_running_pipeline(&pool, project_id).await? {
        return Err(ChatError::ProjectPipelineRunning);
    }

    manager.remove_project(&project).await?;
    Ok(())
}

/// Forces a fresh image build, bypassing the cache-hit check a plain
/// registration would apply for an unchanged source hash - what the
/// projects page's "rebuild" button calls.
#[server(
    auth: crate::auth::server::AuthSession,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn rebuild_project_image(project_id: i64) -> ChatResult<ProjectSummary> {
    crate::auth::operator::require_operator(&auth).await?;

    let project = manager
        .get_project(munibot_ai::project::ProjectId(project_id))
        .await?;
    let rebuilt = manager.rebuild_image(&project).await?;

    let workspace_count = manager.list_workspaces(rebuilt.id).await?.len();
    Ok(summarize(&rebuilt, workspace_count))
}

/// Every registered project, by display name - what the chat page's
/// project picker lists. Any signed-in user, not just an operator: see
/// [`ProjectPickerEntry`]'s own doc comment for why.
#[server(
    auth: crate::auth::server::AuthSession,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn list_projects_for_picker() -> ChatResult<Vec<ProjectPickerEntry>> {
    auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;

    let mut entries: Vec<ProjectPickerEntry> = manager
        .list_projects()
        .await?
        .into_iter()
        .map(|project| ProjectPickerEntry {
            id: project.id.0,
            display_name: project.display_name,
            image_status: project.image_status.label().to_string(),
        })
        .collect();
    entries.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(entries)
}

/// Binds a conversation to a project (or unbinds it, with `project_id:
/// None`) - what the chat page's project picker calls. Owner-gated, not
/// operator-gated: managing a project is an operator's job, but binding
/// one's *own* conversation to an already-registered project is not.
///
/// A bound conversation's `coder` turns provision over that project's own
/// worktree from then on, rather than a fresh scratch workspace every
/// time - see `munibot_ai::service::Ai::resolve_workspace_override`.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    manager: axum::extract::Extension<std::sync::Arc<munibot_ai::project::ProjectManager>>,
)]
pub async fn set_conversation_project(
    conversation_id: i64,
    project_id: Option<i64>,
) -> ChatResult<()> {
    use munibot_core::db::operations::ai;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    crate::server_fns::chat::conversation::owned_conversation(&pool, conversation_id, user.id)
        .await?;

    // validate the project actually exists before binding to it - a
    // stale or made-up id must fail loudly here, not silently produce a
    // conversation bound to nothing the next time a turn looks it up
    if let Some(project_id) = project_id {
        manager
            .get_project(munibot_ai::project::ProjectId(project_id))
            .await?;
    }

    ai::set_conversation_project(&pool, conversation_id, project_id).await?;
    Ok(())
}
