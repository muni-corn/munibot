use dioxus::prelude::*;

/// One tool call in the current turn, as tracked from `ChatEvent::ToolStarted`/
/// `ToolFinished`. `finished` is `None` while the call is still running.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolActivityEntry {
    pub name: String,
    pub finished: Option<ToolActivityResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolActivityResult {
    pub ok: bool,
    pub duration_ms: u64,
    pub result: String,
}

impl ToolActivityEntry {
    /// Starts tracking a new call, not yet finished.
    pub fn started(name: String) -> Self {
        Self {
            name,
            finished: None,
        }
    }
}

/// Records a `ToolStarted` event into `entries`, appending a new
/// not-yet-finished entry.
pub fn record_started(entries: &mut Vec<ToolActivityEntry>, name: String) {
    entries.push(ToolActivityEntry::started(name));
}

/// Records a `ToolFinished` event into `entries`.
///
/// `ChatEvent`'s `ToolStarted`/`ToolFinished` carry only a tool name, not a
/// call id, so a call is matched to the *first* still-unfinished entry with
/// the same name -- exactly right for a single call, or for calls to
/// different tools, and a reasonable best-effort pairing for two concurrent
/// calls to the same tool, since which of an unordered pair is labelled
/// "first" doesn't change what's legible about the strip either way.
pub fn record_finished(entries: &mut [ToolActivityEntry], name: &str, result: ToolActivityResult) {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.name == name && entry.finished.is_none())
    {
        entry.finished = Some(result);
    }
}

/// A live, collapsible strip of the current turn's tool calls, shown above
/// the in-flight reply. Renders nothing while there's nothing to show.
#[component]
pub fn ToolActivityStrip(entries: Vec<ToolActivityEntry>) -> Element {
    if entries.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "mx-4 mb-2 flex flex-col divide-y divide-slate-800 rounded-box bg-slate-900/60",
            for (index, entry) in entries.into_iter().enumerate() {
                ToolActivityRow { key: "{index}", entry }
            }
        }
    }
}

#[component]
fn ToolActivityRow(entry: ToolActivityEntry) -> Element {
    let Some(result) = &entry.finished else {
        return rsx! {
            div { class: "flex items-center gap-2 px-3 py-1.5 text-xs text-slate-400",
                i { class: "animate-spin ph-duotone ph-circle-notch" }
                span { class: "font-mono", {entry.name.clone()} }
                span { class: "opacity-60", "running..." }
            }
        };
    };

    let (icon, icon_class) = if result.ok {
        ("ph-check-circle", "text-success")
    } else {
        ("ph-x-circle", "text-error")
    };

    rsx! {
        details { class: "px-3 py-1.5",
            summary { class: "flex cursor-pointer items-center gap-2 text-xs text-slate-400",
                i { class: "ph-duotone {icon} {icon_class}" }
                span { class: "font-mono", {entry.name.clone()} }
                span { class: "opacity-60", "{result.duration_ms}ms" }
            }
            pre { class: "mt-1 max-h-48 overflow-y-auto rounded bg-slate-950 p-2 text-xs whitespace-pre-wrap text-slate-300",
                {result.result.clone()}
            }
        }
    }
}
