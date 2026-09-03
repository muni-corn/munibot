use dioxus::prelude::*;
use munibot_api::{
    project::{ProjectSummary, RegisterProjectForm},
    server_fns::project::{
        get_project, list_projects, rebuild_project_image, register_project, remove_project,
    },
};

use crate::{app::Route, components::Spinner};

/// Every registered project: a git repository munibot has cloned, one
/// container image, and whatever workspaces are checked out under it.
#[component]
pub fn Projects() -> Element {
    let mut projects = use_resource(move || async move { list_projects().await });

    rsx! {
        document::Title { "projects ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "projects" }
            p { class: "text-sm text-slate-400",
                "every repository munibot has cloned, the image strategy it resolved to, and why any strategy before that one didn't apply."
            }
            RegisterProjectForm2 { on_registered: move |_| projects.restart() }
            match &*projects.read() {
                Some(Ok(list)) if list.is_empty() => rsx! {
                    p { class: "text-slate-400", "no projects registered yet." }
                },
                Some(Ok(list)) => rsx! {
                    div { class: "flex flex-col gap-2",
                        for project in list.iter().cloned() {
                            ProjectRow { project, on_changed: move |_| projects.restart() }
                        }
                    }
                },
                Some(Err(error)) => rsx! {
                    div { class: "alert alert-error", "couldn't load projects :< {error}" }
                },
                None => rsx! {
                    Spinner {}
                },
            }
        }
    }
}

#[component]
fn ProjectRow(project: ProjectSummary, on_changed: EventHandler<()>) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2 rounded-lg bg-slate-900/50 p-4",
            div { class: "flex flex-row items-center gap-4",
                Link {
                    to: Route::ProjectDetail {
                        project_id: project.id,
                    },
                    class: "flex grow flex-row items-center gap-4",
                    span { class: "font-bold", "{project.slug}" }
                    span { class: "text-sm text-slate-400", "{project.repo}" }
                    ImageStatusBadge {
                        strategy: project.image_strategy.clone(),
                        status: project.image_status.clone(),
                    }
                    span { class: "badge badge-neutral", "{project.network_policy}" }
                    span { class: "ml-auto text-xs text-slate-400",
                        "{project.workspace_count} workspace(s)"
                    }
                }
                RebuildButton { project_id: project.id, on_changed }
            }
            if !project.skip_reasons.is_empty() {
                div { class: "flex flex-col gap-1 pl-2 text-xs text-slate-500",
                    for reason in project.skip_reasons.iter() {
                        p { "· {reason}" }
                    }
                }
            }
            if let Some(error) = &project.image_error {
                p { class: "pl-2 text-xs text-red-400", "build failed: {error}" }
            }
        }
    }
}

#[component]
fn ImageStatusBadge(strategy: String, status: String) -> Element {
    let class = match status.as_str() {
        "built" => "badge badge-success",
        "failed" => "badge badge-error",
        _ => "badge badge-neutral",
    };
    rsx! {
        span { class, "{strategy} · {status}" }
    }
}

/// Forces a fresh image build, bypassing the cache-hit check a plain
/// registration would apply for an unchanged source hash.
#[component]
fn RebuildButton(project_id: i64, on_changed: EventHandler<()>) -> Element {
    let mut status = use_signal(|| None::<Result<(), String>>);
    let mut rebuilding = use_signal(|| false);

    let onclick = move |_| {
        spawn(async move {
            rebuilding.set(true);
            let result = rebuild_project_image(project_id)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());
            status.set(Some(result));
            rebuilding.set(false);
            on_changed.call(());
        });
    };

    let label = if *rebuilding.read() {
        "rebuilding…".to_string()
    } else {
        match &*status.read() {
            Some(Err(error)) => format!("failed: {error}"),
            _ => "rebuild".to_string(),
        }
    };

    rsx! {
        button { class: "btn btn-sm", disabled: *rebuilding.read(), onclick, {label} }
    }
}

/// Registers a new project by cloning its repository. No forge token is
/// collected here - only a repository cloneable without one can be
/// registered through this form; a private repository is registered
/// automatically instead, through a GitHub App installation.
#[component]
fn RegisterProjectForm2(on_registered: EventHandler<()>) -> Element {
    let mut slug = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut repo_owner = use_signal(String::new);
    let mut repo_name = use_signal(String::new);
    let mut clone_url = use_signal(String::new);
    let mut default_branch = use_signal(|| "main".to_string());
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);

    let onclick = move |_| {
        spawn(async move {
            submitting.set(true);
            let form = RegisterProjectForm {
                slug: slug.read().clone(),
                display_name: display_name.read().clone(),
                repo_owner: repo_owner.read().clone(),
                repo_name: repo_name.read().clone(),
                clone_url: clone_url.read().clone(),
                default_branch: default_branch.read().clone(),
            };
            match register_project(form).await {
                Ok(_) => {
                    error.set(None);
                    slug.set(String::new());
                    display_name.set(String::new());
                    repo_owner.set(String::new());
                    repo_name.set(String::new());
                    clone_url.set(String::new());
                    on_registered.call(());
                }
                Err(err) => error.set(Some(err.to_string())),
            }
            submitting.set(false);
        });
    };

    rsx! {
        div { class: "flex flex-col gap-2 rounded-lg bg-slate-900/50 p-4",
            p { class: "font-bold", "register a project" }
            div { class: "flex flex-wrap gap-2",
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "slug",
                    value: "{slug}",
                    oninput: move |e| slug.set(e.value()),
                }
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "display name",
                    value: "{display_name}",
                    oninput: move |e| display_name.set(e.value()),
                }
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "repo owner",
                    value: "{repo_owner}",
                    oninput: move |e| repo_owner.set(e.value()),
                }
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "repo name",
                    value: "{repo_name}",
                    oninput: move |e| repo_name.set(e.value()),
                }
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "clone url",
                    value: "{clone_url}",
                    oninput: move |e| clone_url.set(e.value()),
                }
                input {
                    class: "input input-sm input-bordered",
                    placeholder: "default branch",
                    value: "{default_branch}",
                    oninput: move |e| default_branch.set(e.value()),
                }
                button {
                    class: "btn btn-primary btn-sm",
                    disabled: *submitting.read(),
                    onclick,
                    if *submitting.read() {
                        "registering…"
                    } else {
                        "register"
                    }
                }
            }
            if let Some(message) = error.read().as_ref() {
                p { class: "text-xs text-red-400", "couldn't register :< {message}" }
            }
        }
    }
}

/// One project's own summary, its workspaces, and a way to remove it
/// entirely.
#[component]
pub fn ProjectDetail(project_id: i64) -> Element {
    let detail = use_resource(move || async move { get_project(project_id).await });

    let content = match &*detail.read() {
        Some(Ok(detail)) => {
            let summary = detail.summary.clone();
            let workspaces = detail.workspaces.clone();
            rsx! {
                div { class: "flex flex-col gap-4",
                    div { class: "flex flex-row items-center gap-4",
                        span { class: "font-bold", "{summary.slug}" }
                        span { class: "text-sm text-slate-400", "{summary.repo}" }
                        ImageStatusBadge {
                            strategy: summary.image_strategy.clone(),
                            status: summary.image_status.clone(),
                        }
                    }
                    p { class: "text-sm text-slate-400", "network: {summary.network_policy}" }
                    if let Some(trigger) = &summary.trigger {
                        p { class: "text-sm text-slate-400", "trigger: {trigger}" }
                    }
                    if !summary.skip_reasons.is_empty() {
                        div { class: "flex flex-col gap-1 text-xs text-slate-500",
                            for reason in summary.skip_reasons.iter() {
                                p { "· {reason}" }
                            }
                        }
                    }
                    div { class: "flex flex-col gap-1",
                        p { class: "font-bold", "workspaces" }
                        for workspace in workspaces.iter() {
                            div { class: "flex flex-row gap-4 rounded bg-slate-900/50 p-2 font-mono text-xs",
                                span { "{workspace.name}" }
                                span { class: "text-slate-500", "{workspace.branch}" }
                                if let Some(pipeline_id) = workspace.pipeline_id {
                                    span { class: "badge badge-neutral", "pipeline #{pipeline_id}" }
                                }
                            }
                        }
                    }
                    RemoveProjectButton { project_id: summary.id }
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load that project :< {e}" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "project ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            Link { to: Route::Projects {}, "← back to projects" }
            {content}
        }
    }
}

/// Removes a project entirely: every worktree, the bare clone, and the
/// row itself. Refused server-side while a pipeline is still running
/// against it - this button surfaces that error rather than preventing
/// the attempt, since only the server actually knows.
#[component]
fn RemoveProjectButton(project_id: i64) -> Element {
    let mut status = use_signal(|| None::<String>);
    let mut removing = use_signal(|| false);
    let nav = use_navigator();

    let onclick = move |_| {
        spawn(async move {
            removing.set(true);
            match remove_project(project_id).await {
                Ok(()) => {
                    nav.push(Route::Projects {});
                }
                Err(error) => status.set(Some(error.to_string())),
            }
            removing.set(false);
        });
    };

    rsx! {
        div { class: "flex flex-col gap-1",
            button {
                class: "btn w-fit btn-error btn-sm",
                disabled: *removing.read(),
                onclick,
                if *removing.read() {
                    "removing…"
                } else {
                    "remove project"
                }
            }
            if let Some(message) = status.read().as_ref() {
                p { class: "text-xs text-red-400", "couldn't remove :< {message}" }
            }
        }
    }
}
