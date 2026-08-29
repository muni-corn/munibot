use serde::{Deserialize, Serialize};

mod breakdown;

pub use breakdown::{UsageBreakdown, UsageBreakdownEntry};

/// Aggregate totals over some slice of `ai_usage` - either the signed-in
/// user's own history, or the whole service's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct UsageTotals {
    pub cost_micros: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub turn_count: i64,
}

#[cfg(feature = "server")]
impl From<munibot_core::db::operations::ai::UsageTotals> for UsageTotals {
    fn from(totals: munibot_core::db::operations::ai::UsageTotals) -> Self {
        Self {
            cost_micros: totals.cost_micros,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            turn_count: totals.turn_count,
        }
    }
}

/// One scope's spend against its configured cap, as shown in a usage panel.
/// Absent entirely (see `UsageSummary::spend_cap`) rather than zeroed out
/// when no cap is configured - "uncapped" and "at zero of a tiny cap" are
/// different things a panel should render differently.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SpendCapStatus {
    pub limit_micros: i64,
    pub current_micros: i64,
    /// RFC 3339, pre-formatted - the same reasoning
    /// `munibot_ai::limits::SpendCapError`'s own `reset_at` field documents.
    pub reset_at: String,
    pub period: String,
}

#[cfg(feature = "server")]
impl From<munibot_ai::limits::SpendCapStatus> for SpendCapStatus {
    fn from(status: munibot_ai::limits::SpendCapStatus) -> Self {
        Self {
            limit_micros: status.limit_micros,
            current_micros: status.current_micros,
            reset_at: status.reset_at.to_rfc3339(),
            period: status.period,
        }
    }
}

/// What a usage panel shows: what you have spent (`totals`, all-time),
/// against what you are allowed to spend (`spend_cap`, if anything is
/// configured for this scope).
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct UsageSummary {
    pub totals: UsageTotals,
    pub spend_cap: Option<SpendCapStatus>,
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    #[test]
    fn test_usage_totals_converts_field_for_field() {
        let core_totals = munibot_core::db::operations::ai::UsageTotals {
            cost_micros: 1_000,
            input_tokens: 10,
            output_tokens: 20,
            turn_count: 3,
        };
        let totals: UsageTotals = core_totals.into();
        assert_eq!(totals, UsageTotals {
            cost_micros: 1_000,
            input_tokens: 10,
            output_tokens: 20,
            turn_count: 3,
        });
    }

    #[test]
    fn test_spend_cap_status_formats_reset_at_as_rfc3339() {
        let reset_at: DateTime<Utc> = "2026-08-06T12:00:00Z".parse().unwrap();
        let status: SpendCapStatus = munibot_ai::limits::SpendCapStatus {
            limit_micros: 5_000_000,
            current_micros: 1_000_000,
            reset_at,
            period: "monthly".to_string(),
        }
        .into();
        assert_eq!(status.reset_at, "2026-08-06T12:00:00+00:00");
        assert_eq!(status.period, "monthly");
    }

    #[test]
    fn test_default_usage_summary_has_no_spend_cap() {
        let summary = UsageSummary::default();
        assert!(summary.spend_cap.is_none());
        assert_eq!(summary.totals, UsageTotals::default());
    }
}
