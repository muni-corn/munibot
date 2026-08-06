use std::time::Duration;

use serde_json::Value;

use crate::types::{AiError, Cost, Usage};

/// One increment of progress through a harness turn, meant for rendering to a
/// user: a Discord message being edited, a status line, a progress indicator.
///
/// A platform adapter is the only thing that should ever need to inspect these;
/// nothing else in the system reaches into the loop's own state directly.
#[derive(Debug)]
pub enum HarnessEvent {
    /// The turn has begun.
    ///
    /// Carries the persona's name as a plain string, not a persona type: the
    /// harness must not depend on the persona module, since personas depend
    /// on the harness rather than the other way around.
    TurnStarted { persona: String },
    /// A chunk of the model's reasoning, where the provider exposes it.
    Thinking(String),
    /// A chunk of assistant-visible text.
    TextDelta(String),
    /// A tool call has begun.
    ToolStarted { name: String },
    /// A tool call has finished.
    ToolFinished {
        name: String,
        duration: Duration,
        ok: bool,
        /// The tool's own success text, recoverable error, or fatal error
        /// message - the same text an audit record keeps, carried out here
        /// too so a live consumer (a chat page's tool activity display) can
        /// show it without waiting for the turn to end.
        result: String,
    },
    /// One provider round trip has finished.
    IterationComplete { iteration: usize, usage: Usage },
    /// The model produced a structured handoff payload.
    Handoff(Value),
    /// The turn is over.
    TurnFinished { usage: Usage, cost: Cost },
    /// The turn failed.
    Failed(AiError),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_turn_started_carries_the_persona_name() {
        let event = HarnessEvent::TurnStarted {
            persona: "companion".to_string(),
        };
        match event {
            HarnessEvent::TurnStarted { persona } => assert_eq!(persona, "companion"),
            other => panic!("expected TurnStarted, got {other:?}"),
        }
    }

    #[test]
    fn test_tool_finished_carries_its_outcome() {
        let event = HarnessEvent::ToolFinished {
            name: "web_search".to_string(),
            duration: Duration::from_millis(250),
            ok: true,
            result: "three results found".to_string(),
        };
        match event {
            HarnessEvent::ToolFinished {
                name,
                duration,
                ok,
                result,
            } => {
                assert_eq!(name, "web_search");
                assert_eq!(duration, Duration::from_millis(250));
                assert!(ok);
                assert_eq!(result, "three results found");
            }
            other => panic!("expected ToolFinished, got {other:?}"),
        }
    }

    #[test]
    fn test_handoff_carries_the_raw_payload() {
        let event = HarnessEvent::Handoff(json!({"action": "ApprovePlan"}));
        match event {
            HarnessEvent::Handoff(payload) => assert_eq!(payload["action"], "ApprovePlan"),
            other => panic!("expected Handoff, got {other:?}"),
        }
    }

    #[test]
    fn test_turn_finished_carries_usage_and_cost() {
        let event = HarnessEvent::TurnFinished {
            usage: Usage::new(10, 20),
            cost: Cost::from_micros(500),
        };
        match event {
            HarnessEvent::TurnFinished { usage, cost } => {
                assert_eq!(usage, Usage::new(10, 20));
                assert_eq!(cost, Cost::from_micros(500));
            }
            other => panic!("expected TurnFinished, got {other:?}"),
        }
    }

    #[test]
    fn test_failed_carries_the_error() {
        let event = HarnessEvent::Failed(AiError::Cancelled);
        assert!(matches!(event, HarnessEvent::Failed(AiError::Cancelled)));
    }
}
