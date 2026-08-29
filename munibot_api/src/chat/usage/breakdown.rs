use serde::{Deserialize, Serialize};

use crate::chat::UsageTotals;

/// One grouped slice of usage - a persona, a `provider:model` pair, a user,
/// or a day - alongside its own totals.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UsageBreakdownEntry {
    /// A persona id, a `"provider:model"` string, a user id (as a plain
    /// string - this dto carries no display name, since resolving one
    /// means a second query per row this view has no other need for), or
    /// an ISO 8601 date (`"2026-07-30"`, parseable by whatever chart
    /// eventually renders `UsageBreakdown::daily`).
    pub label: String,
    pub totals: UsageTotals,
}

/// The operator dashboard's own view: global usage broken down four ways,
/// each already sorted highest-cost-first (or, for `daily`, oldest first) -
/// see `munibot_core::db::operations::ai::sum_usage_by_persona` and its
/// siblings for why the sorting happens in rust rather than sql.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct UsageBreakdown {
    pub by_persona: Vec<UsageBreakdownEntry>,
    pub by_model: Vec<UsageBreakdownEntry>,
    pub by_user: Vec<UsageBreakdownEntry>,
    pub daily: Vec<UsageBreakdownEntry>,
}

#[cfg(feature = "server")]
mod convert {
    use munibot_core::db::operations::ai;

    use super::{UsageBreakdown, UsageBreakdownEntry};

    fn entry<K: ToString>((key, totals): (K, ai::UsageTotals)) -> UsageBreakdownEntry {
        UsageBreakdownEntry {
            label: key.to_string(),
            totals: totals.into(),
        }
    }

    impl UsageBreakdown {
        /// Assembles a full breakdown from each grouped query's own
        /// result. A free-standing constructor rather than a `From` impl,
        /// since there is no single core type this maps from - the four
        /// queries are independent, only ever combined here.
        pub fn assemble(
            by_persona: Vec<(String, ai::UsageTotals)>,
            by_model: Vec<((String, String), ai::UsageTotals)>,
            by_user: Vec<(Option<i64>, ai::UsageTotals)>,
            daily: Vec<(chrono::NaiveDate, ai::UsageTotals)>,
        ) -> Self {
            Self {
                by_persona: by_persona.into_iter().map(entry).collect(),
                by_model: by_model
                    .into_iter()
                    .map(|((provider, model), totals)| {
                        entry((format!("{provider}:{model}"), totals))
                    })
                    .collect(),
                by_user: by_user
                    .into_iter()
                    .map(|(user_id, totals)| {
                        entry((
                            user_id.map_or_else(|| "unknown".to_string(), |id| id.to_string()),
                            totals,
                        ))
                    })
                    .collect(),
                daily: daily
                    .into_iter()
                    .map(|(date, totals)| entry((date.to_string(), totals)))
                    .collect(),
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use munibot_core::db::operations::ai::UsageTotals as CoreUsageTotals;

    use super::*;

    #[test]
    fn test_assemble_formats_model_labels_as_provider_colon_model() {
        let breakdown = UsageBreakdown::assemble(
            Vec::new(),
            vec![(
                ("anthropic".to_string(), "claude-opus-5".to_string()),
                CoreUsageTotals::default(),
            )],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(breakdown.by_model[0].label, "anthropic:claude-opus-5");
    }

    #[test]
    fn test_assemble_labels_a_missing_user_as_unknown() {
        let breakdown = UsageBreakdown::assemble(
            Vec::new(),
            Vec::new(),
            vec![(None, CoreUsageTotals::default())],
            Vec::new(),
        );
        assert_eq!(breakdown.by_user[0].label, "unknown");
    }

    #[test]
    fn test_assemble_formats_a_present_user_id_as_its_own_string() {
        let breakdown = UsageBreakdown::assemble(
            Vec::new(),
            Vec::new(),
            vec![(Some(42), CoreUsageTotals::default())],
            Vec::new(),
        );
        assert_eq!(breakdown.by_user[0].label, "42");
    }

    #[test]
    fn test_assemble_formats_daily_labels_as_iso_dates() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
        let breakdown = UsageBreakdown::assemble(Vec::new(), Vec::new(), Vec::new(), vec![(
            date,
            CoreUsageTotals::default(),
        )]);
        assert_eq!(breakdown.daily[0].label, "2026-07-30");
    }
}
