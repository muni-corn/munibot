use dioxus::prelude::*;
use munibot_api::{
    pipeline::PipelineSummary,
    server_fns::pipeline::{get_pipeline_detail, pipeline_monitor_stream},
};

use crate::{app::Route, components::Spinner};

/// Live list of every pipeline run munibot has ever started, streamed over
/// server-sent events. An unobservable autonomous system is an unusable
/// one -- this is the whole reason the pipeline is watchable from
/// anywhere at all, not just from wherever it happened to be started.
#[component]
pub fn Pipelines() -> Element {
    let mut pipelines = use_signal(Vec::<PipelineSummary>::new);
    let mut error = use_signal(|| None::<String>);

    use_future(move || async move {
        match pipeline_monitor_stream().await {
            Ok(mut stream) => {
                while let Some(snapshot) = stream.recv().await {
                    match snapshot {
                        Ok(snapshot) => {
                            error.set(None);
                            pipelines.set(snapshot);
                        }
                        Err(e) => error.set(Some(e.to_string())),
                    }
                }
            }
            Err(e) => error.set(Some(e.to_string())),
        }
    });

    rsx! {
        document::Title { "pipelines ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "text-2xl font-black", "pipelines" }
            p { class: "text-sm text-slate-400",
                "every autonomous pipeline run, updated live. nothing here ever merges on its own."
            }
            if let Some(message) = error.read().as_ref() {
                div { class: "alert alert-error", "couldn't load pipelines :< {message}" }
            }
            if pipelines.read().is_empty() && error.read().is_none() {
                Spinner {}
            }
            div { class: "flex flex-col gap-2",
                for pipeline in pipelines.read().iter().cloned() {
                    PipelineRow { pipeline }
                }
            }
        }
    }
}

#[component]
fn PipelineRow(pipeline: PipelineSummary) -> Element {
    rsx! {
        Link {
            to: Route::PipelineDetail {
                pipeline_id: pipeline.id,
            },
            class: "flex flex-row items-center gap-4 rounded-lg bg-slate-900/50 p-4 hover:bg-slate-900",
            span { class: "font-mono text-sm text-slate-400", "#{pipeline.id}" }
            span { class: "font-bold", "{pipeline.owner}/{pipeline.repo_name}#{pipeline.issue_number}" }
            StateBadge {
                state: pipeline.state.clone(),
                subtask: pipeline.subtask.clone(),
            }
            if pipeline.running {
                span { class: "badge badge-success", "running" }
            }
            span { class: "ml-auto font-mono text-xs text-slate-400",
                {format_elapsed(pipeline.elapsed_seconds)}
            }
        }
    }
}

#[component]
fn StateBadge(state: String, subtask: Option<String>) -> Element {
    let label = match subtask {
        Some(subtask) => format!("{state} · {subtask}"),
        None => state,
    };
    rsx! {
        span { class: "badge badge-neutral", {label} }
    }
}

/// One pipeline's own summary and full event log.
#[component]
pub fn PipelineDetail(pipeline_id: i64) -> Element {
    let detail = use_resource(move || async move { get_pipeline_detail(pipeline_id).await });

    let content = match &*detail.read() {
        Some(Ok(detail)) => {
            let summary = detail.summary.clone();
            rsx! {
                div { class: "flex flex-col gap-4",
                    div { class: "flex flex-row items-center gap-4",
                        span { class: "font-bold",
                            "{summary.owner}/{summary.repo_name}#{summary.issue_number}"
                        }
                        StateBadge {
                            state: summary.state.clone(),
                            subtask: summary.subtask.clone(),
                        }
                    }
                    p { class: "text-sm text-slate-400",
                        "elapsed: {format_elapsed(summary.elapsed_seconds)}"
                    }
                    // "every agent invocation" is exactly one row per
                    // event, since each event is what one agent's own
                    // handoff produced -- per-tool-call detail within a
                    // turn is not wired into this view yet
                    div { class: "flex flex-col gap-1",
                        for event in detail.events.iter() {
                            div { class: "flex flex-row gap-4 rounded bg-slate-900/50 p-2 font-mono text-xs",
                                span { class: "text-slate-500", "#{event.seq}" }
                                span { {event.event_type.clone()} }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load that pipeline :< {e}" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    rsx! {
        document::Title { "pipeline #{pipeline_id} ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            Link { to: Route::Pipelines {}, "← back to pipelines" }
            {content}
        }
    }
}

fn format_elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_elapsed_shows_seconds_under_a_minute() {
        assert_eq!(format_elapsed(42), "42s");
    }

    #[test]
    fn test_format_elapsed_shows_minutes_under_an_hour() {
        assert_eq!(format_elapsed(125), "2m");
    }

    #[test]
    fn test_format_elapsed_shows_hours_and_minutes() {
        assert_eq!(format_elapsed(3725), "1h2m");
    }

    #[test]
    fn test_format_elapsed_never_goes_negative() {
        assert_eq!(format_elapsed(-5), "0s");
    }
}
