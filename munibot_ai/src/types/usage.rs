use std::{
    iter::Sum,
    ops::{Add, AddAssign},
};

use serde::{Deserialize, Serialize};

/// Tokens consumed by one or more requests.
///
/// Implements [`Add`] and [`Sum`] so several requests can be totalled without
/// the caller tracking the counters by hand.
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
    /// Tokens spent on internal reasoning by thinking-capable models (Anthropic
    /// extended thinking, OpenAI's o-series, Gemini thinking). Billed as
    /// output on every provider we target, but kept separate so thinking can be
    /// audited on its own.
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl Usage {
    /// Builds a usage record from input and output counts, with no caching or
    /// reasoning tokens.
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
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
            + self.reasoning_tokens
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
            reasoning_tokens: self.reasoning_tokens + other.reasoning_tokens,
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
            reasoning_tokens: 50,
        };
        let b = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
            reasoning_tokens: 5,
        };

        let total = a + b;

        assert_eq!(total.input_tokens, 11, "input tokens should add");
        assert_eq!(total.output_tokens, 22, "output tokens should add");
        assert_eq!(total.cache_read_tokens, 33, "cache reads should add");
        assert_eq!(total.cache_write_tokens, 44, "cache writes should add");
        assert_eq!(total.reasoning_tokens, 55, "reasoning tokens should add");
    }

    #[test]
    fn test_usage_sums_over_an_iterator() {
        let total: Usage = vec![Usage::new(1, 2), Usage::new(3, 4), Usage::new(5, 6)]
            .into_iter()
            .sum();
        assert_eq!(
            total,
            Usage::new(9, 12),
            "several requests should be able to total themselves"
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
            reasoning_tokens: 16,
        };
        assert_eq!(
            usage.total_tokens(),
            31,
            "every token, including reasoning, should count toward the total"
        );
    }

    #[test]
    fn test_usage_deserializes_with_missing_cache_fields() {
        // providers without prompt caching or extended thinking omit these
        // entirely
        let usage: Usage =
            serde_json::from_value(serde_json::json!({"input_tokens": 10, "output_tokens": 5}))
                .expect("should deserialize without cache or reasoning fields");
        assert_eq!(
            usage,
            Usage::new(10, 5),
            "missing cache and reasoning counts should be zero"
        );
    }
}
