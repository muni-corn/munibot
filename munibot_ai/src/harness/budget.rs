use std::time::{Duration, Instant};

use crate::types::{AiError, Cost, Usage};

/// Limits on one agent turn.
///
/// Every field is optional, so a persona only states what it cares about.
/// [`Budget::default`] is deliberately conservative rather than unlimited: an
/// unconfigured limit and no limit at all are not the same thing, and a
/// misconfigured persona should fail cheaply rather than run away.
#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// Total provider round trips, including the first.
    pub max_iterations: Option<usize>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock: Option<Duration>,
    pub max_cost: Option<Cost>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_iterations: Some(8),
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock: Some(Duration::from_secs(60)),
            max_cost: Some(Cost::from_dollars(0.25)),
        }
    }
}

/// Accumulates usage, cost, and elapsed time against a [`Budget`] over the
/// course of one turn.
///
/// The harness loop checks this at the top of every iteration, before spending
/// anything on it, and again after recording that iteration's usage - so a turn
/// that goes over mid-iteration is caught immediately rather than only at the
/// start of the next one.
pub struct BudgetTracker {
    budget: Budget,
    started_at: Instant,
    iterations: usize,
    usage: Usage,
    cost: Cost,
}

impl BudgetTracker {
    /// Starts tracking against `budget`, with the clock starting now.
    pub fn new(budget: Budget) -> Self {
        Self {
            budget,
            started_at: Instant::now(),
            iterations: 0,
            usage: Usage::default(),
            cost: Cost::ZERO,
        }
    }

    /// Records one completed iteration's usage and cost.
    pub fn record(&mut self, usage: Usage, cost: Cost) {
        self.iterations += 1;
        self.usage += usage;
        self.cost += cost;
    }

    /// Checks every configured limit, returning the first one exceeded.
    pub fn check(&self) -> Result<(), AiError> {
        if let Some(max) = self.budget.max_iterations
            && self.iterations >= max
        {
            return Err(Self::exceeded(format!("{max} iterations")));
        }
        if let Some(max) = self.budget.max_input_tokens
            && self.usage.input_tokens >= max
        {
            return Err(Self::exceeded(format!("{max} input tokens")));
        }
        if let Some(max) = self.budget.max_output_tokens
            && self.usage.output_tokens >= max
        {
            return Err(Self::exceeded(format!("{max} output tokens")));
        }
        if let Some(max) = self.budget.max_wall_clock
            && self.started_at.elapsed() >= max
        {
            return Err(Self::exceeded(format!("{max:?} wall clock")));
        }
        if let Some(max) = self.budget.max_cost
            && self.cost >= max
        {
            return Err(Self::exceeded(format!("{max} cost")));
        }
        Ok(())
    }

    fn exceeded(limit: String) -> AiError {
        AiError::BudgetExceeded { limit }
    }

    /// Every iteration's usage, summed so far.
    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// Every iteration's cost, summed so far.
    pub fn cost(&self) -> Cost {
        self.cost
    }

    /// How many iterations have been recorded so far.
    pub fn iterations(&self) -> usize {
        self.iterations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlimited() -> Budget {
        Budget {
            max_iterations: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock: None,
            max_cost: None,
        }
    }

    #[test]
    fn test_default_budget_is_conservative() {
        let budget = Budget::default();
        assert_eq!(budget.max_iterations, Some(8));
        assert_eq!(budget.max_wall_clock, Some(Duration::from_secs(60)));
        assert_eq!(budget.max_cost, Some(Cost::from_dollars(0.25)));
        assert_eq!(
            budget.max_input_tokens, None,
            "token limits are left to the cost ceiling by default"
        );
    }

    #[test]
    fn test_unlimited_budget_never_triggers() {
        let mut tracker = BudgetTracker::new(unlimited());
        for _ in 0..1000 {
            tracker.record(Usage::new(1_000_000, 1_000_000), Cost::from_dollars(100.0));
        }
        assert!(
            tracker.check().is_ok(),
            "a budget with every limit unset should never trip"
        );
    }

    #[test]
    fn test_check_passes_before_any_iteration() {
        let tracker = BudgetTracker::new(Budget::default());
        assert!(
            tracker.check().is_ok(),
            "a fresh tracker should not have exceeded anything yet"
        );
    }

    #[test]
    fn test_iteration_limit_trips_once_reached() {
        let budget = Budget {
            max_iterations: Some(2),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);

        tracker.record(Usage::default(), Cost::ZERO);
        assert!(
            tracker.check().is_ok(),
            "one of two iterations should not trip the limit"
        );

        tracker.record(Usage::default(), Cost::ZERO);
        assert!(
            tracker.check().is_err(),
            "the second of a two-iteration budget should trip it"
        );
    }

    #[test]
    fn test_input_token_limit_trips_once_reached() {
        let budget = Budget {
            max_input_tokens: Some(100),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);

        tracker.record(Usage::new(99, 0), Cost::ZERO);
        assert!(tracker.check().is_ok());

        tracker.record(Usage::new(1, 0), Cost::ZERO);
        assert!(tracker.check().is_err());
    }

    #[test]
    fn test_output_token_limit_trips_once_reached() {
        let budget = Budget {
            max_output_tokens: Some(100),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);

        tracker.record(Usage::new(0, 100), Cost::ZERO);
        assert!(tracker.check().is_err());
    }

    #[test]
    fn test_cost_limit_trips_once_reached() {
        let budget = Budget {
            max_cost: Some(Cost::from_dollars(1.0)),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);

        tracker.record(Usage::default(), Cost::from_dollars(1.0));
        assert!(tracker.check().is_err());
    }

    #[test]
    fn test_wall_clock_limit_trips_immediately_when_zero() {
        // any elapsed time at all exceeds a zero limit, so this is deterministic
        // without needing to actually wait on the real clock
        let budget = Budget {
            max_wall_clock: Some(Duration::ZERO),
            ..unlimited()
        };
        let tracker = BudgetTracker::new(budget);
        assert!(
            tracker.check().is_err(),
            "a zero wall clock budget should trip immediately"
        );
    }

    #[test]
    fn test_record_accumulates_across_calls() {
        let mut tracker = BudgetTracker::new(unlimited());
        tracker.record(Usage::new(10, 20), Cost::from_micros(100));
        tracker.record(Usage::new(5, 5), Cost::from_micros(50));

        assert_eq!(tracker.usage(), Usage::new(15, 25));
        assert_eq!(tracker.cost(), Cost::from_micros(150));
        assert_eq!(tracker.iterations(), 2);
    }

    #[test]
    fn test_exceeded_error_names_the_specific_limit() {
        let budget = Budget {
            max_iterations: Some(1),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);
        tracker.record(Usage::default(), Cost::ZERO);

        let error = tracker.check().expect_err("should have tripped");
        assert!(
            error.to_string().contains("iterations"),
            "the error should say which limit was hit, got {error}"
        );
    }
}
