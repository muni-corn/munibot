use dioxus::prelude::*;
use munibot_api::{
    chat::{SpendCapStatus, UsageSummary},
    server_fns::chat::usage::{get_global_usage, get_my_usage},
};

use crate::components::{Spinner, settings::SettingsSection};

/// What you have spent, against what you are allowed to spend - for
/// yourself always, and service-wide too if you hold `Permission::Operator`.
///
/// Showing people their own cost is both honest and the cheapest possible
/// abuse deterrent - see the milestone plan's own reasoning for this page.
/// The service-wide section simply doesn't render for anyone `get_global_usage`
/// refuses, rather than showing an error: not being an operator is normal,
/// not a failure.
#[component]
pub fn Usage() -> Element {
    let mine = use_resource(get_my_usage);
    let global = use_resource(get_global_usage);

    let mine_content = match &*mine.read() {
        Some(Ok(summary)) => rsx! {
            UsageCard { summary: summary.clone() }
        },
        Some(Err(e)) => rsx! {
            div { class: "alert alert-error", "couldn't load your usage :< {e}" }
        },
        None => rsx! {
            Spinner {}
        },
    };

    // a NotOperator refusal here is expected for almost everyone, not an
    // error worth showing - this section just never appears for them
    let global_content = match &*global.read() {
        Some(Ok(summary)) => rsx! {
            SettingsSection {
                title: "service-wide".to_string(),
                description: Some("every user, combined.".to_string()),
                UsageCard { summary: summary.clone() }
            }
        },
        _ => rsx! {},
    };

    rsx! {
        document::Title { "usage ~ munibot" }
        div { class: "flex h-full flex-col gap-4 p-6",
            h2 { class: "font-black text-2xl", "usage" }
            SettingsSection {
                title: "your usage".to_string(),
                description: Some("what you've spent talking with munibot.".to_string()),
                {mine_content}
            }
            {global_content}
        }
    }
}

#[component]
fn UsageCard(summary: UsageSummary) -> Element {
    rsx! {
        div { class: "flex flex-col gap-4",
            div { class: "flex flex-wrap gap-6",
                Stat {
                    label: "spent",
                    value: format_cost(summary.totals.cost_micros),
                }
                Stat {
                    label: "turns",
                    value: summary.totals.turn_count.to_string(),
                }
                Stat {
                    label: "tokens",
                    value: (summary.totals.input_tokens + summary.totals.output_tokens).to_string(),
                }
            }
            if let Some(cap) = &summary.spend_cap {
                SpendCapBar { cap: cap.clone() }
            }
        }
    }
}

#[component]
fn Stat(label: String, value: String) -> Element {
    rsx! {
        div { class: "flex flex-col",
            span { class: "text-xs uppercase tracking-wide text-slate-400", {label} }
            span { class: "font-mono font-bold text-lg", {value} }
        }
    }
}

#[component]
fn SpendCapBar(cap: SpendCapStatus) -> Element {
    let ratio = if cap.limit_micros > 0 {
        (cap.current_micros as f64 / cap.limit_micros as f64).min(1.0)
    } else {
        0.0
    };
    let percent = (ratio * 100.0).round();
    let bar_class = if ratio >= 1.0 {
        "bg-error"
    } else if ratio >= 0.8 {
        "bg-warning"
    } else {
        "bg-primary"
    };

    rsx! {
        div { class: "flex flex-col gap-1",
            div { class: "flex items-center justify-between text-sm text-slate-400",
                span {
                    "{format_cost(cap.current_micros)} of {format_cost(cap.limit_micros)} this {cap.period}"
                }
                span { "resets {cap.reset_at}" }
            }
            div { class: "rounded-full bg-slate-800 h-2 w-full overflow-hidden",
                div {
                    class: "h-full rounded-full {bar_class}",
                    style: "width: {percent}%",
                }
            }
        }
    }
}

/// Renders whole-cent dollars from micros - `$1.23` rather than
/// `$1.2345` or a raw micro count nobody reads at a glance.
fn format_cost(micros: i64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_cost_renders_two_decimal_dollars() {
        assert_eq!(format_cost(1_234_567), "$1.23");
        assert_eq!(format_cost(0), "$0.00");
        assert_eq!(format_cost(500_000), "$0.50");
    }
}
