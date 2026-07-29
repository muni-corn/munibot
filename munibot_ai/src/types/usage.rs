use std::{
    fmt,
    iter::Sum,
    ops::{Add, AddAssign},
};

use serde::{Deserialize, Serialize};

/// Tokens consumed by one or more requests.
///
/// Implements [`Add`] and [`Sum`] so a multi-iteration agent loop can total
/// itself without the caller tracking four counters by hand.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens in the prompt, excluding anything served from cache.
    #[serde(default)]
    pub input_tokens: u64,
    /// Tokens generated in the response.
    #[serde(default)]
    pub output_tokens: u64,
    /// Prompt tokens served from a provider-side cache, usually billed at a
    /// discount.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Prompt tokens written into a provider-side cache, usually billed at a
    /// premium.
    #[serde(default)]
    pub cache_write_tokens: u64,
}

impl Usage {
    /// Builds a usage record from input and output counts, with no caching.
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            ..Self::default()
        }
    }

    /// Every token accounted for, cached or not.
    ///
    /// Useful for budget checks, where the question is how much context was
    /// moved rather than what it cost.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

impl Add for Usage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cache_read_tokens: self.cache_read_tokens + other.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens + other.cache_write_tokens,
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl Sum for Usage {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), |total, next| total + next)
    }
}

/// Money, in micro-dollars.
///
/// Deliberately an integer. These values are summed across thousands of
/// requests and stored in the database, and floating point rounding across that
/// many additions produces spend figures that do not reconcile. One
/// micro-dollar is a millionth of a dollar, which is finer than any provider
/// prices at.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct Cost(pub i64);

impl Cost {
    /// No cost.
    pub const ZERO: Self = Self(0);

    /// Builds a cost from micro-dollars.
    pub fn from_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// Builds a cost from whole dollars, for configuration and budget ceilings.
    pub fn from_dollars(dollars: f64) -> Self {
        Self((dollars * 1_000_000.0).round() as i64)
    }

    /// The raw micro-dollar count.
    pub fn micros(&self) -> i64 {
        self.0
    }

    /// The cost in dollars, for display only. Never sum these.
    pub fn as_dollars(&self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
}

impl Add for Cost {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl AddAssign for Cost {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl Sum for Cost {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self(iter.map(|cost| cost.0).sum())
    }
}

impl fmt::Display for Cost {
    /// Formats as dollars with four decimal places, which is enough to show a
    /// single cheap turn.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.4}", self.as_dollars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_adds_every_field() {
        let a = Usage {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 30,
            cache_write_tokens: 40,
        };
        let b = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
        };

        let total = a + b;

        assert_eq!(total.input_tokens, 11, "input tokens should add");
        assert_eq!(total.output_tokens, 22, "output tokens should add");
        assert_eq!(total.cache_read_tokens, 33, "cache reads should add");
        assert_eq!(total.cache_write_tokens, 44, "cache writes should add");
    }

    #[test]
    fn test_usage_sums_over_an_iterator() {
        let total: Usage = vec![Usage::new(1, 2), Usage::new(3, 4), Usage::new(5, 6)]
            .into_iter()
            .sum();
        assert_eq!(
            total,
            Usage::new(9, 12),
            "an agent loop should be able to total itself"
        );
    }

    #[test]
    fn test_usage_add_assign_accumulates() {
        let mut running = Usage::default();
        running += Usage::new(5, 5);
        running += Usage::new(5, 5);
        assert_eq!(
            running,
            Usage::new(10, 10),
            "add assign should accumulate in place"
        );
    }

    #[test]
    fn test_total_tokens_counts_cache_traffic() {
        let usage = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 4,
            cache_write_tokens: 8,
        };
        assert_eq!(
            usage.total_tokens(),
            15,
            "every token should count toward the total"
        );
    }

    #[test]
    fn test_usage_deserializes_with_missing_cache_fields() {
        // providers without prompt caching omit these entirely
        let usage: Usage =
            serde_json::from_value(serde_json::json!({"input_tokens": 10, "output_tokens": 5}))
                .expect("should deserialize without cache fields");
        assert_eq!(
            usage,
            Usage::new(10, 5),
            "missing cache counts should be zero"
        );
    }

    #[test]
    fn test_cost_from_dollars_converts_to_micros() {
        assert_eq!(
            Cost::from_dollars(1.0),
            Cost::from_micros(1_000_000),
            "a dollar should be a million micros"
        );
        assert_eq!(
            Cost::from_dollars(0.25),
            Cost::from_micros(250_000),
            "a quarter should be 250000 micros"
        );
    }

    #[test]
    fn test_cost_from_dollars_rounds_rather_than_truncates() {
        // 0.0000005 dollars is half a micro; truncating would silently lose it
        assert_eq!(
            Cost::from_dollars(0.0000005),
            Cost::from_micros(1),
            "sub-micro amounts should round, not truncate to zero"
        );
    }

    #[test]
    fn test_cost_sums_exactly() {
        // the point of integer micros: a thousand additions must reconcile precisely
        let one_third_cent = Cost::from_micros(3_333);
        let total: Cost = std::iter::repeat_n(one_third_cent, 1_000).sum();
        assert_eq!(
            total,
            Cost::from_micros(3_333_000),
            "summing many costs must be exact"
        );
    }

    #[test]
    fn test_cost_displays_as_dollars() {
        assert_eq!(
            Cost::from_micros(12_345).to_string(),
            "$0.0123",
            "cost should display as dollars for humans"
        );
    }

    #[test]
    fn test_cost_orders_numerically() {
        assert!(
            Cost::from_micros(100) > Cost::from_micros(99),
            "costs must be comparable so budgets can be enforced"
        );
        assert_eq!(Cost::ZERO, Cost::from_micros(0), "zero should be zero");
    }

    #[test]
    fn test_cost_serializes_as_a_bare_integer() {
        let encoded = serde_json::to_value(Cost::from_micros(500)).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!(500),
            "cost should store as an integer column, not an object"
        );
    }
}
