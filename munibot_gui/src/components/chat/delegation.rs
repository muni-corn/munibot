use dioxus::prelude::*;

/// One delegation in the current turn, as tracked from
/// `ChatEvent::DelegationStarted`/`DelegationFinished`. `finished` is `None`
/// while the specialist is still working.
///
/// Delegation must never be invisible - that was the entire objection to
/// automatic persona routing - so this renders distinctly from a plain
/// [`crate::components::chat::tool_activity::ToolActivityEntry`], not folded
/// into the same strip.
///
/// Deliberately shows only that a specialist was asked and whether they
/// finished, not their own tool activity nested underneath: the harness does
/// not yet forward a delegated turn's own internal tool events back through
/// the outer stream (`Ai::delegate` runs a plain, non-streamed
/// `Harness::run_turn`), so there is nothing to nest yet. The specialist's
/// actual answer reaches the person through the companion's own reply, which
/// is the point - he reports back in his own voice rather than the chat page
/// showing the specialist's raw output directly.
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationEntry {
    pub persona: String,
    pub task: String,
    pub finished: Option<bool>,
}

impl DelegationEntry {
    pub fn started(persona: String, task: String) -> Self {
        Self {
            persona,
            task,
            finished: None,
        }
    }
}

/// Records a `DelegationStarted` event into `entries`, appending a new
/// not-yet-finished entry.
pub fn record_delegation_started(
    entries: &mut Vec<DelegationEntry>,
    persona: String,
    task: String,
) {
    entries.push(DelegationEntry::started(persona, task));
}

/// Records a `DelegationFinished` event into `entries`.
///
/// Matched to the *first* still-unfinished entry for that persona, the same
/// best-effort pairing `tool_activity::record_finished` uses and for the
/// same reason: `ChatEvent` carries no call id to pair against exactly.
pub fn record_delegation_finished(entries: &mut [DelegationEntry], persona: &str, ok: bool) {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.persona == persona && entry.finished.is_none())
    {
        entry.finished = Some(ok);
    }
}

/// A live strip of the current turn's delegations, shown above the in-flight
/// reply. Renders nothing while there are none.
#[component]
pub fn DelegationStrip(entries: Vec<DelegationEntry>) -> Element {
    if entries.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "flex flex-col mx-4 mb-2 gap-1",
            for (index, entry) in entries.into_iter().enumerate() {
                DelegationCard { key: "{index}", entry }
            }
        }
    }
}

#[component]
fn DelegationCard(entry: DelegationEntry) -> Element {
    let status = match entry.finished {
        None => rsx! {
            i { class: "animate-spin ph-duotone ph-circle-notch text-info" }
        },
        Some(true) => rsx! {
            i { class: "ph-duotone ph-check-circle text-success" }
        },
        Some(false) => rsx! {
            i { class: "ph-duotone ph-x-circle text-error" }
        },
    };

    rsx! {
        div { class: "flex items-start gap-2 rounded-box border border-info/30 bg-info/10 px-3 py-2 text-xs",
            div { class: "mt-0.5", {status} }
            div { class: "flex flex-col",
                span { class: "font-semibold text-info", "asked {entry.persona} for help" }
                span { class: "text-slate-400", {entry.task.clone()} }
            }
        }
    }
}
