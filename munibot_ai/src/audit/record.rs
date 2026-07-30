use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::{ConversationId, ToolOutcome};

/// How long an audited field is kept before being truncated.
///
/// Generous enough to be useful for debugging a bad tool loop, small enough
/// that one chatty tool call cannot bloat the audit table - the same
/// trade-off munibot's own length limits elsewhere in this crate make for
/// the same reason.
const MAX_FIELD_LEN: usize = 2000;

/// What one finished tool call is worth remembering.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallRecord {
    pub conversation_id: Option<ConversationId>,
    pub tool_name: String,
    /// The arguments the model called the tool with, truncated to
    /// [`MAX_FIELD_LEN`] characters.
    pub input: String,
    /// What the tool returned - its success text, its recoverable error, or
    /// its fatal error's message - truncated the same way.
    pub output: String,
    pub duration: Duration,
    pub status: ToolCallStatus,
}

impl ToolCallRecord {
    /// Builds a record from a tool's outcome, truncating both fields to
    /// [`MAX_FIELD_LEN`] characters.
    pub fn from_outcome(
        conversation_id: ConversationId,
        tool_name: &str,
        input: &Value,
        outcome: &ToolOutcome,
        duration: Duration,
    ) -> Self {
        let (output, status) = match outcome {
            ToolOutcome::Ok(text) => (text.clone(), ToolCallStatus::Ok),
            ToolOutcome::Err(text) => (text.clone(), ToolCallStatus::Err),
            ToolOutcome::Fatal(error) => (error.to_string(), ToolCallStatus::Fatal),
        };

        Self {
            conversation_id: Some(conversation_id),
            tool_name: tool_name.to_string(),
            input: truncate(&input.to_string(), MAX_FIELD_LEN),
            output: truncate(&output, MAX_FIELD_LEN),
            duration,
            status,
        }
    }
}

/// How a tool call ended, stored as a short, stable string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolCallStatus {
    /// The tool succeeded.
    Ok,
    /// The tool failed in a way the model can recover from.
    Err,
    /// The tool failed in a way that aborted the whole turn.
    Fatal,
}

impl ToolCallStatus {
    /// The stable string this status is stored as, mirroring
    /// [`crate::types::Role::as_key`].
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
            Self::Fatal => "fatal",
        }
    }
}

/// Truncates `text` to at most `max_len` characters on a character boundary,
/// marking the cut with a trailing ellipsis.
fn truncate(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    if max_len == 0 {
        return String::new();
    }
    let mut truncated: String = text.chars().take(max_len - 1).collect();
    truncated.push('…');
    truncated
}

/// Records one finished tool call.
///
/// Must never propagate a failure to the caller - [`crate::harness::Harness`]
/// calls this as a side effect of dispatching a tool, and auditing failing to
/// write must never affect whether the tool call itself succeeded. An
/// implementation that can fail (a database write, say) should log and
/// swallow its own error, the same way [`crate::usage::UsageRecorder`]'s
/// callers do at their own call site.
#[async_trait]
pub trait ToolAuditor: Send + Sync {
    async fn record(&self, record: ToolCallRecord);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::AiError;

    #[test]
    fn test_from_outcome_maps_ok_to_the_ok_status() {
        let record = ToolCallRecord::from_outcome(
            ConversationId(1),
            "current_time",
            &json!({}),
            &ToolOutcome::Ok("12:00".to_string()),
            Duration::from_millis(5),
        );
        assert_eq!(record.status, ToolCallStatus::Ok);
        assert_eq!(record.output, "12:00");
    }

    #[test]
    fn test_from_outcome_maps_err_to_the_err_status() {
        let record = ToolCallRecord::from_outcome(
            ConversationId(1),
            "search",
            &json!({}),
            &ToolOutcome::Err("invalid arguments".to_string()),
            Duration::from_millis(5),
        );
        assert_eq!(record.status, ToolCallStatus::Err);
        assert_eq!(record.output, "invalid arguments");
    }

    #[test]
    fn test_from_outcome_maps_fatal_to_the_fatal_status_using_the_errors_message() {
        let record = ToolCallRecord::from_outcome(
            ConversationId(1),
            "bash",
            &json!({}),
            &ToolOutcome::Fatal(AiError::Cancelled),
            Duration::from_millis(5),
        );
        assert_eq!(record.status, ToolCallStatus::Fatal);
        assert!(record.output.contains("cancelled"));
    }

    #[test]
    fn test_from_outcome_carries_the_conversation_id() {
        let record = ToolCallRecord::from_outcome(
            ConversationId(42),
            "current_time",
            &json!({}),
            &ToolOutcome::Ok("12:00".to_string()),
            Duration::from_millis(5),
        );
        assert_eq!(record.conversation_id, Some(ConversationId(42)));
    }

    #[test]
    fn test_from_outcome_truncates_a_long_input_and_output() {
        let long_output = "a".repeat(MAX_FIELD_LEN + 100);
        let record = ToolCallRecord::from_outcome(
            ConversationId(1),
            "web_fetch",
            &json!({"url": "a".repeat(MAX_FIELD_LEN + 100)}),
            &ToolOutcome::Ok(long_output),
            Duration::from_millis(5),
        );
        assert_eq!(record.output.chars().count(), MAX_FIELD_LEN);
        assert!(record.output.ends_with('…'));
        assert!(record.input.chars().count() <= MAX_FIELD_LEN);
    }

    #[test]
    fn test_short_input_and_output_are_not_truncated() {
        let record = ToolCallRecord::from_outcome(
            ConversationId(1),
            "current_time",
            &json!({"timezone": "UTC"}),
            &ToolOutcome::Ok("12:00".to_string()),
            Duration::from_millis(5),
        );
        assert_eq!(record.output, "12:00");
        assert!(!record.output.ends_with('…'));
    }

    #[test]
    fn test_status_as_key_gives_stable_short_strings() {
        assert_eq!(ToolCallStatus::Ok.as_key(), "ok");
        assert_eq!(ToolCallStatus::Err.as_key(), "err");
        assert_eq!(ToolCallStatus::Fatal.as_key(), "fatal");
    }

    #[test]
    fn test_truncate_is_multibyte_safe() {
        let text = "é".repeat(10);
        let truncated = truncate(&text, 3);
        assert_eq!(truncated.chars().count(), 3);
    }

    #[test]
    fn test_truncate_leaves_short_text_untouched() {
        assert_eq!(truncate("hi", 10), "hi");
    }
}
