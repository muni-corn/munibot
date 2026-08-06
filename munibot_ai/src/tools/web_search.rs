use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    tools::{
        RiskTier, Tool, ToolCtx, ToolOutcome,
        exa::{ContentsOptions, ExaBackend, TextOptions},
        wrap_untrusted,
    },
    types::ToolSchema,
};

/// How much of each result's extracted text to keep. Generous enough for a
/// model to work with, bounded so one search cannot dominate a turn's context
/// budget.
const MAX_CHARACTERS_PER_RESULT: usize = 2000;

#[derive(Deserialize, JsonSchema)]
struct WebSearchArgs {
    /// What to search for.
    query: String,
    /// How many results to return. Defaults to 5 when omitted.
    num_results: Option<usize>,
}

/// Searches the web via Exa.
///
/// Requests highlights and text inline in the same round trip, so a research
/// loop does not need a separate fetch per result - see finding 8 in
/// `docs/notes/ai-preflight-findings.md`. Results are attacker-reachable
/// content (anyone can put anything on a web page a search might surface), so
/// the whole response goes out through [`wrap_untrusted`].
///
/// Generic over [`ExaBackend`] rather than holding a concrete `ExaClient`, so
/// it can be unit-tested with a scripted fake and no network access.
pub struct WebSearchTool {
    backend: Arc<dyn ExaBackend>,
}

impl WebSearchTool {
    /// Builds the tool over any Exa backend, real or scripted.
    pub fn new(backend: Arc<dyn ExaBackend>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Searches the web and returns titles, URLs, and relevant excerpts for each result. Use \
         this to find current information, verify a claim, or discover sources on a topic."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::NetworkRead
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<WebSearchArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let args: WebSearchArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };

        let contents = ContentsOptions {
            highlights: Some(true),
            text: Some(TextOptions {
                max_characters: Some(MAX_CHARACTERS_PER_RESULT),
            }),
        };

        // an Exa outage should not end the conversation - the model can carry on
        // without search, so this is a recoverable error, not a Fatal one
        let response = match self
            .backend
            .search(args.query.clone(), args.num_results, Some(contents))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return ToolOutcome::err(format!("web search is unavailable right now :< {error}"));
            }
        };

        if response.results.is_empty() {
            return ToolOutcome::ok(format!("no results found for {:?}", args.query));
        }

        let formatted = response
            .results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                let title = result.title.as_deref().unwrap_or("(untitled)");
                let snippet = result
                    .highlights
                    .first()
                    .map(String::as_str)
                    .or(result.text.as_deref())
                    .unwrap_or("(no excerpt available)");
                format!("{}. {title}\n   {}\n   {snippet}", index + 1, result.url)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        ToolOutcome::ok(wrap_untrusted("web_search", &formatted))
    }
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::*;
    use crate::tools::{
        ConversationId, Platform,
        exa::{CostDollars, ExaError, ExaResult, FakeExaBackend, SearchResponse},
    };

    fn ctx() -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier: RiskTier::NetworkRead,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
        }
    }

    fn result(url: &str, title: &str, highlight: &str) -> ExaResult {
        ExaResult {
            id: url.to_string(),
            title: Some(title.to_string()),
            url: url.to_string(),
            published_date: None,
            author: None,
            text: Some(highlight.to_string()),
            highlights: vec![highlight.to_string()],
            image: None,
            favicon: None,
        }
    }

    fn tool_with(backend: FakeExaBackend) -> WebSearchTool {
        WebSearchTool::new(Arc::new(backend))
    }

    #[test]
    fn test_tool_metadata() {
        let tool = tool_with(FakeExaBackend::respond_search(Ok(SearchResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        assert_eq!(tool.name(), "web_search");
        assert_eq!(tool.tier(), RiskTier::NetworkRead);
    }

    #[test]
    fn test_input_schema_requires_only_query() {
        let tool = tool_with(FakeExaBackend::respond_search(Ok(SearchResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        let schema = tool.input_schema();
        assert!(schema["properties"].get("query").is_some());
        assert!(schema["properties"].get("num_results").is_some());
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert_eq!(required, vec![serde_json::json!("query")]);
    }

    #[tokio::test]
    async fn test_results_are_formatted_and_wrapped_as_untrusted() {
        let tool = tool_with(FakeExaBackend::respond_search(Ok(SearchResponse {
            request_id: "r1".to_string(),
            results: vec![result("https://tokio.rs/", "Tokio", "an async runtime")],
            cost_dollars: Some(CostDollars { total: 0.007 }),
        })));

        let outcome = tool
            .invoke(serde_json::json!({"query": "tokio"}), &ctx())
            .await;

        match outcome {
            ToolOutcome::Ok(text) => {
                assert!(
                    text.contains("<untrusted-content"),
                    "results must be wrapped: {text:?}"
                );
                assert!(text.contains("Tokio"));
                assert!(text.contains("https://tokio.rs/"));
                assert!(text.contains("an async runtime"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_empty_results_are_reported_plainly() {
        let tool = tool_with(FakeExaBackend::respond_search(Ok(SearchResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));

        let outcome = tool
            .invoke(serde_json::json!({"query": "asdkjfhaslkdjfh"}), &ctx())
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("no results")),
            other => panic!("expected success reporting no results, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_backend_failure_is_recoverable_not_fatal() {
        // an Exa outage should let the model carry on without search, not abort the
        // turn
        let tool = tool_with(FakeExaBackend::respond_search(Err(ExaError::Api {
            status: StatusCode::UNAUTHORIZED,
            message: "Invalid API key (INVALID_API_KEY)".to_string(),
        })));

        let outcome = tool
            .invoke(serde_json::json!({"query": "cats"}), &ctx())
            .await;
        match outcome {
            ToolOutcome::Err(message) => {
                assert!(message.contains("unavailable"), "got {message:?}")
            }
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let tool = tool_with(FakeExaBackend::respond_search(Ok(SearchResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        let outcome = tool.invoke(serde_json::json!({}), &ctx()).await;
        assert!(
            matches!(outcome, ToolOutcome::Err(_)),
            "missing required query should be rejected"
        );
    }
}
