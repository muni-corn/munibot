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
    /// Total tool calls across the whole turn that may fail schema validation
    /// before the turn gives up entirely. Separate from `max_iterations`,
    /// since a model stuck retrying malformed arguments should not have to
    /// burn a full provider round trip's worth of budget per attempt
    /// to find that out.
    pub max_tool_retries: Option<usize>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_iterations: Some(8),
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock: Some(Duration::from_secs(60)),
            max_cost: Some(Cost::from_dollars(0.25)),
            max_tool_retries: Some(3),
        }
    }
}

impl Budget {
    /// The tighter of this budget and `other`, field by field.
    ///
    /// `None` means unbounded, so it never wins over a real limit on either
    /// side - only `None` paired with `None` stays `None`. Used to cap a
    /// delegated turn's own persona-configured budget by whatever the
    /// enclosing turn actually has left ([`BudgetTracker::remaining`]),
    /// without ever widening it past its own configured ceiling either: a
    /// cheap reviewer persona should stay cheap even when the enclosing
    /// turn has plenty of budget left to give it.
    pub fn bounded_by(&self, other: &Budget) -> Budget {
        fn tighter<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            }
        }

        Budget {
            max_iterations: tighter(self.max_iterations, other.max_iterations),
            max_input_tokens: tighter(self.max_input_tokens, other.max_input_tokens),
            max_output_tokens: tighter(self.max_output_tokens, other.max_output_tokens),
            max_wall_clock: tighter(self.max_wall_clock, other.max_wall_clock),
            max_cost: tighter(self.max_cost, other.max_cost),
            max_tool_retries: tighter(self.max_tool_retries, other.max_tool_retries),
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

    /// This turn's own budget, minus whatever has already been spent - what
    /// a delegated turn should be bounded by, rather than the specialist
    /// persona's full configured budget regardless of how much of the
    /// enclosing turn has already run.
    ///
    /// Every field saturates at zero rather than going negative: a limit
    /// already exceeded (checked separately by [`Self::check`], before this
    /// is ever consulted) means nothing at all is left, not a negative
    /// allowance.
    pub fn remaining(&self) -> Budget {
        Budget {
            max_iterations: self
                .budget
                .max_iterations
                .map(|max| max.saturating_sub(self.iterations)),
            max_input_tokens: self
                .budget
                .max_input_tokens
                .map(|max| max.saturating_sub(self.usage.input_tokens)),
            max_output_tokens: self
                .budget
                .max_output_tokens
                .map(|max| max.saturating_sub(self.usage.output_tokens)),
            max_wall_clock: self
                .budget
                .max_wall_clock
                .map(|max| max.saturating_sub(self.started_at.elapsed())),
            max_cost: self
                .budget
                .max_cost
                .map(|max| Cost((max.0 - self.cost.0).max(0))),
            // tool retries aren't tracked by BudgetTracker itself (the harness
            // loop counts them separately), so there is nothing spent to
            // subtract here - a delegated turn gets the same retry allowance
            // the enclosing one was configured with
            max_tool_retries: self.budget.max_tool_retries,
        }
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
            max_tool_retries: None,
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
        assert_eq!(budget.max_tool_retries, Some(3));
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

    #[test]
    fn test_remaining_of_a_fresh_tracker_is_the_full_budget() {
        let budget = Budget {
            max_iterations: Some(8),
            max_cost: Some(Cost::from_dollars(0.25)),
            ..unlimited()
        };
        let tracker = BudgetTracker::new(budget.clone());
        let remaining = tracker.remaining();
        assert_eq!(remaining.max_iterations, Some(8));
        assert_eq!(remaining.max_cost, Some(Cost::from_dollars(0.25)));
    }

    #[test]
    fn test_remaining_subtracts_what_has_been_spent() {
        let budget = Budget {
            max_iterations: Some(8),
            max_cost: Some(Cost::from_micros(1000)),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);
        tracker.record(Usage::new(10, 20), Cost::from_micros(300));

        let remaining = tracker.remaining();
        assert_eq!(remaining.max_iterations, Some(7));
        assert_eq!(remaining.max_cost, Some(Cost::from_micros(700)));
    }

    #[test]
    fn test_remaining_saturates_at_zero_rather_than_going_negative() {
        let budget = Budget {
            max_iterations: Some(1),
            max_cost: Some(Cost::from_micros(100)),
            ..unlimited()
        };
        let mut tracker = BudgetTracker::new(budget);
        tracker.record(Usage::default(), Cost::from_micros(500));
        tracker.record(Usage::default(), Cost::ZERO);

        let remaining = tracker.remaining();
        assert_eq!(remaining.max_iterations, Some(0));
        assert_eq!(remaining.max_cost, Some(Cost::ZERO));
    }

    #[test]
    fn test_remaining_keeps_unset_fields_unset() {
        let tracker = BudgetTracker::new(unlimited());
        let remaining = tracker.remaining();
        assert_eq!(remaining, unlimited());
    }

    #[test]
    fn test_bounded_by_takes_the_tighter_of_two_set_limits() {
        let persona_budget = Budget {
            max_iterations: Some(20),
            ..unlimited()
        };
        let remaining = Budget {
            max_iterations: Some(3),
            ..unlimited()
        };
        assert_eq!(
            persona_budget.bounded_by(&remaining).max_iterations,
            Some(3)
        );
    }

    #[test]
    fn test_bounded_by_does_not_widen_past_its_own_ceiling() {
        // a cheap persona should stay cheap even when the enclosing turn has
        // plenty of budget left to hand it
        let persona_budget = Budget {
            max_cost: Some(Cost::from_micros(100)),
            ..unlimited()
        };
        let remaining = Budget {
            max_cost: Some(Cost::from_dollars(100.0)),
            ..unlimited()
        };
        assert_eq!(
            persona_budget.bounded_by(&remaining).max_cost,
            Some(Cost::from_micros(100))
        );
    }

    #[test]
    fn test_bounded_by_an_unbounded_other_keeps_its_own_limit() {
        let persona_budget = Budget {
            max_iterations: Some(5),
            ..unlimited()
        };
        assert_eq!(
            persona_budget.bounded_by(&unlimited()).max_iterations,
            Some(5)
        );
    }

    #[test]
    fn test_an_unbounded_self_adopts_the_others_limit() {
        let remaining = Budget {
            max_iterations: Some(5),
            ..unlimited()
        };
        assert_eq!(unlimited().bounded_by(&remaining).max_iterations, Some(5));
    }

    #[test]
    fn test_bounded_by_stays_unlimited_when_both_are() {
        assert_eq!(unlimited().bounded_by(&unlimited()), unlimited());
    }

    #[test]
    fn test_remaining_keeps_the_same_tool_retry_allowance() {
        let budget = Budget {
            max_tool_retries: Some(3),
            ..unlimited()
        };
        let tracker = BudgetTracker::new(budget);
        assert_eq!(tracker.remaining().max_tool_retries, Some(3));
    }
}
