use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const EXA_API_BASE: &str = "https://api.exa.ai";

/// User-Agent sent with every Exa API request, following the pattern in
/// `munibot_discord/src/pluralkit/api.rs`.
const USER_AGENT: &str = concat!(
    "munibot/",
    env!("CARGO_PKG_VERSION"),
    " (https://git.musicaloft.com/municorn/munibot)",
);

/// How long to wait for Exa to respond before giving up.
///
/// A search or fetch tool call sits inside a persona's own turn budget, so a
/// hung request must not be allowed to consume the whole thing silently.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Requests text extraction alongside search results or a content fetch.
///
/// Verified against the live API: `/search` and `/contents` both accept this
/// shape inline, so a search and its content extraction can happen in one round
/// trip rather than two.
#[derive(Serialize, Clone, Debug, Default)]
pub struct ContentsOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlights: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextOptions>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct TextOptions {
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxCharacters")]
    pub max_characters: Option<usize>,
}

#[derive(Serialize, Clone, Debug)]
struct SearchRequest {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "numResults")]
    num_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contents: Option<ContentsOptions>,
}

/// One result from a search or a content fetch.
///
/// Fields beyond `url` are all optional: Exa does not guarantee every field is
/// populated for every result, and a partially-empty result is still useful
/// rather than something to reject.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ExaResult {
    pub id: String,
    pub title: Option<String>,
    pub url: String,
    #[serde(rename = "publishedDate")]
    pub published_date: Option<String>,
    pub author: Option<String>,
    pub text: Option<String>,
    #[serde(default)]
    pub highlights: Vec<String>,
    pub image: Option<String>,
    pub favicon: Option<String>,
}

/// The dollar cost Exa reports for one call, letting search spend be recorded
/// exactly rather than estimated - see finding 8 in
/// `docs/notes/ai-preflight-findings.md`.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct CostDollars {
    pub total: f64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SearchResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub results: Vec<ExaResult>,
    #[serde(rename = "costDollars")]
    pub cost_dollars: Option<CostDollars>,
}

#[derive(Serialize, Clone, Debug)]
struct ContentsRequest {
    urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<TextOptions>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ContentsResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub results: Vec<ExaResult>,
    #[serde(rename = "costDollars")]
    pub cost_dollars: Option<CostDollars>,
}

/// The JSON error envelope Exa returns on a non-success response, verified
/// live: `{"error": "Invalid API key", "tag": "INVALID_API_KEY"}` at HTTP 401.
#[derive(Deserialize, Clone, Debug)]
struct ExaErrorBody {
    error: String,
    tag: Option<String>,
}

/// Errors from talking to the Exa API.
#[derive(Error, Debug)]
pub enum ExaError {
    #[error("request to exa failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("exa rejected the request ({status}): {message}")]
    Api { status: StatusCode, message: String },
    #[error("exa isn't configured: {0}")]
    NotConfigured(String),
}

/// The subset of [`ExaClient`] the built-in search and fetch tools depend on.
///
/// Exists so those tools can be unit-tested with a scripted fake rather than
/// the real client - matching how [`crate::provider::MockProvider`] lets the
/// harness be tested with no network access. `ExaClient` implements this
/// directly.
#[async_trait::async_trait]
pub trait ExaBackend: Send + Sync {
    async fn search(
        &self,
        query: String,
        num_results: Option<usize>,
        contents: Option<ContentsOptions>,
    ) -> Result<SearchResponse, ExaError>;

    async fn contents(
        &self,
        urls: Vec<String>,
        text: Option<TextOptions>,
    ) -> Result<ContentsResponse, ExaError>;
}

/// A client for the Exa search and content extraction API.
///
/// Wraps a `reqwest::Client` carrying the API key and a descriptive User-Agent,
/// following the pattern in `munibot_discord/src/pluralkit/api.rs`.
#[derive(Clone)]
pub struct ExaClient {
    client: Client,
    api_key: String,
}

impl std::fmt::Debug for ExaClient {
    /// Redacts the API key, so a stray `{:?}` in a log line or a test failure
    /// message cannot leak it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExaClient")
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl ExaClient {
    /// Builds a client from an explicit API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            // reqwest only fails to build a Client when TLS initialization fails, which would be
            // a fatal startup error anyway
            .expect("failed to build reqwest client for exa");

        Self {
            client,
            api_key: api_key.into(),
        }
    }

    /// Builds a client from the `EXA_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, ExaError> {
        let api_key = std::env::var("EXA_API_KEY")
            .map_err(|_| ExaError::NotConfigured("EXA_API_KEY is not set".to_string()))?;
        Ok(Self::new(api_key))
    }

    /// Searches the web, optionally requesting text and highlights inline so a
    /// caller does not need a follow-up [`Self::contents`] call per result.
    pub async fn search(
        &self,
        query: impl Into<String>,
        num_results: Option<usize>,
        contents: Option<ContentsOptions>,
    ) -> Result<SearchResponse, ExaError> {
        let body = SearchRequest {
            query: query.into(),
            num_results,
            contents,
        };
        self.post("/search", &body).await
    }

    /// Fetches the extracted content of specific URLs directly, for links the
    /// user supplied rather than ones a search turned up.
    pub async fn contents(
        &self,
        urls: Vec<String>,
        text: Option<TextOptions>,
    ) -> Result<ContentsResponse, ExaError> {
        let body = ContentsRequest { urls, text };
        self.post("/contents", &body).await
    }

    async fn post<B: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, ExaError> {
        let response = self
            .client
            .post(format!("{EXA_API_BASE}{path}"))
            .header("x-api-key", &self.api_key)
            .json(body)
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            Ok(response.json::<R>().await?)
        } else {
            let message = match response.json::<ExaErrorBody>().await {
                Ok(body) => match body.tag {
                    Some(tag) => format!("{} ({tag})", body.error),
                    None => body.error,
                },
                Err(_) => format!("http status {status}"),
            };
            Err(ExaError::Api { status, message })
        }
    }
}

#[async_trait::async_trait]
impl ExaBackend for ExaClient {
    async fn search(
        &self,
        query: String,
        num_results: Option<usize>,
        contents: Option<ContentsOptions>,
    ) -> Result<SearchResponse, ExaError> {
        ExaClient::search(self, query, num_results, contents).await
    }

    async fn contents(
        &self,
        urls: Vec<String>,
        text: Option<TextOptions>,
    ) -> Result<ContentsResponse, ExaError> {
        ExaClient::contents(self, urls, text).await
    }
}

/// A scripted [`ExaBackend`] for tests elsewhere in this crate - the same role
/// [`crate::provider::MockProvider`] plays for the harness. Not gated inside
/// `mod tests`, so `web_search`'s and `web_fetch`'s own test modules can use it
/// too.
#[cfg(test)]
pub(crate) struct FakeExaBackend {
    search_result: std::sync::Mutex<Option<Result<SearchResponse, ExaError>>>,
    contents_result: std::sync::Mutex<Option<Result<ContentsResponse, ExaError>>>,
}

#[cfg(test)]
impl FakeExaBackend {
    pub(crate) fn respond_search(result: Result<SearchResponse, ExaError>) -> Self {
        Self {
            search_result: std::sync::Mutex::new(Some(result)),
            contents_result: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn respond_contents(result: Result<ContentsResponse, ExaError>) -> Self {
        Self {
            search_result: std::sync::Mutex::new(None),
            contents_result: std::sync::Mutex::new(Some(result)),
        }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl ExaBackend for FakeExaBackend {
    async fn search(
        &self,
        _query: String,
        _num_results: Option<usize>,
        _contents: Option<ContentsOptions>,
    ) -> Result<SearchResponse, ExaError> {
        self.search_result
            .lock()
            .unwrap()
            .take()
            .expect("FakeExaBackend::search called without a scripted response")
    }

    async fn contents(
        &self,
        _urls: Vec<String>,
        _text: Option<TextOptions>,
    ) -> Result<ContentsResponse, ExaError> {
        self.contents_result
            .lock()
            .unwrap()
            .take()
            .expect("FakeExaBackend::contents called without a scripted response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixtures below are real response bodies captured from the live API during
    // development, not hand-written guesses - see
    // docs/notes/ai-preflight-findings.md

    const SEARCH_RESPONSE_FIXTURE: &str = r#"{
        "requestId": "354abe670677fdebabd2d73123b39f1c",
        "resolvedSearchType": "",
        "results": [
            {
                "id": "https://tokio.rs/",
                "title": "Tokio - An asynchronous Rust runtime",
                "url": "https://tokio.rs/",
                "publishedDate": "2026-05-29T21:14:52.000Z",
                "text": "Tokio - An asynchronous Rust runtime...",
                "highlights": ["Tokio is an asynchronous runtime..."],
                "image": "https://tokio.rs/img/tokio-horizontal.svg",
                "favicon": "https://tokio.rs/favicon-32x32.png"
            },
            {
                "id": "https://docs.rs/tokio/latest/tokio/runtime/index.html",
                "title": "tokio::runtime - Rust",
                "url": "https://docs.rs/tokio/latest/tokio/runtime/index.html",
                "text": "tokio::runtime - Rust...",
                "highlights": ["The Tokio runtime..."],
                "favicon": "https://docs.rs/-/rustdoc.static/favicon-32x32-eab170b8.png"
            }
        ],
        "searchTime": 775.5,
        "costDollars": {
            "total": 0.007,
            "search": {"neural": 0.007}
        }
    }"#;

    const CONTENTS_RESPONSE_FIXTURE: &str = r#"{
        "requestId": "82e8537778c3bafc9629e4d88c0aaac5",
        "results": [
            {
                "id": "https://tokio.rs/",
                "title": "Tokio - An asynchronous Rust runtime",
                "url": "https://tokio.rs/",
                "publishedDate": "2026-05-29T21:14:52.000Z",
                "author": null,
                "text": "Tokio - An asynchronous Rust runtime...",
                "image": "https://tokio.rs/img/tokio-horizontal.svg",
                "favicon": "https://tokio.rs/favicon-32x32.png"
            }
        ],
        "statuses": [
            {"id": "https://tokio.rs/", "status": "success", "source": "cached"}
        ],
        "costDollars": {
            "total": 0.001,
            "contents": {"text": 0.001}
        },
        "searchTime": 8.403266000008443
    }"#;

    const ERROR_RESPONSE_FIXTURE: &str = r#"{
        "requestId": "b3601a5ad9dba51e9b7f63e332fee8e0",
        "error": "Invalid API key",
        "tag": "INVALID_API_KEY"
    }"#;

    #[test]
    fn test_search_response_deserializes_from_the_real_api_shape() {
        let response: SearchResponse =
            serde_json::from_str(SEARCH_RESPONSE_FIXTURE).expect("should deserialize");

        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].url, "https://tokio.rs/");
        assert_eq!(response.results[0].highlights.len(), 1);
        assert_eq!(
            response.cost_dollars.expect("should have a cost").total,
            0.007
        );
    }

    #[test]
    fn test_search_response_tolerates_a_result_missing_optional_fields() {
        // the second fixture result has no image field at all
        let response: SearchResponse =
            serde_json::from_str(SEARCH_RESPONSE_FIXTURE).expect("should deserialize");
        assert_eq!(response.results[1].image, None);
    }

    #[test]
    fn test_contents_response_deserializes_from_the_real_api_shape() {
        let response: ContentsResponse =
            serde_json::from_str(CONTENTS_RESPONSE_FIXTURE).expect("should deserialize");

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].author, None,
            "a null author should deserialize to None"
        );
        assert_eq!(
            response.cost_dollars.expect("should have a cost").total,
            0.001
        );
    }

    #[test]
    fn test_error_body_deserializes_from_the_real_api_shape() {
        let body: ExaErrorBody =
            serde_json::from_str(ERROR_RESPONSE_FIXTURE).expect("should deserialize");
        assert_eq!(body.error, "Invalid API key");
        assert_eq!(body.tag.as_deref(), Some("INVALID_API_KEY"));
    }

    #[test]
    fn test_search_request_omits_absent_optional_fields() {
        let request = SearchRequest {
            query: "cats".to_string(),
            num_results: None,
            contents: None,
        };
        let encoded = serde_json::to_value(&request).expect("should serialize");
        assert_eq!(encoded, serde_json::json!({"query": "cats"}));
    }

    #[test]
    fn test_search_request_uses_exa_camel_case_field_names() {
        let request = SearchRequest {
            query: "cats".to_string(),
            num_results: Some(5),
            contents: Some(ContentsOptions {
                highlights: Some(true),
                text: Some(TextOptions {
                    max_characters: Some(500),
                }),
            }),
        };
        let encoded = serde_json::to_value(&request).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!({
                "query": "cats",
                "numResults": 5,
                "contents": {"highlights": true, "text": {"maxCharacters": 500}}
            })
        );
    }

    #[test]
    fn test_contents_request_serializes_urls_and_text_options() {
        let request = ContentsRequest {
            urls: vec!["https://example.com".to_string()],
            text: Some(TextOptions {
                max_characters: Some(1000),
            }),
        };
        let encoded = serde_json::to_value(&request).expect("should serialize");
        assert_eq!(
            encoded,
            serde_json::json!({
                "urls": ["https://example.com"],
                "text": {"maxCharacters": 1000}
            })
        );
    }

    #[test]
    fn test_from_env_fails_clearly_when_key_is_unset() {
        // reads the process environment, so this cannot run concurrently with a test
        // that depends on EXA_API_KEY being set - none currently do, since no
        // test in this suite touches the network (see the crate's testing
        // policy) SAFETY: single-threaded with respect to this variable within
        // this test process; no other test reads or writes EXA_API_KEY
        unsafe {
            std::env::remove_var("EXA_API_KEY");
        }
        let error = ExaClient::from_env().expect_err("should fail without a key");
        assert!(matches!(error, ExaError::NotConfigured(_)));
    }
}
