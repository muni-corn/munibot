use std::{net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::{
    tools::{
        RiskTier, Tool, ToolCtx, ToolOutcome,
        exa::{ExaBackend, TextOptions},
        wrap_untrusted,
    },
    types::ToolSchema,
};

const DEFAULT_MAX_CHARACTERS: usize = 4000;
const HTML_WRAP_WIDTH: usize = 100;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

const USER_AGENT: &str = concat!(
    "munibot/",
    env!("CARGO_PKG_VERSION"),
    " (https://git.musicaloft.com/municorn/munibot)",
);

#[derive(Deserialize, JsonSchema)]
struct WebFetchArgs {
    /// The URL to fetch. Must be `http` or `https`.
    url: String,
    /// Maximum characters of extracted text to return. Defaults to 4000.
    max_characters: Option<usize>,
}

/// Fetches and extracts the readable content of one URL.
///
/// Prefers Exa's own `contents` endpoint, which does extraction server-side and
/// is what `web_search` already uses - falling back to a direct fetch plus
/// basic HTML-to-text conversion only when Exa cannot or will not serve it.
/// Every result goes out through [`wrap_untrusted`]: an arbitrary URL supplied
/// by the user or found by a search is exactly the kind of attacker-authored
/// content that tool exists for.
///
/// # Security
///
/// [`validate_fetch_url`] rejects non-`http(s)` schemes and resolves the host
/// to reject requests aimed at private, loopback, link-local, or other
/// non-public address ranges - blocking the obvious server-side request forgery
/// targets (cloud metadata endpoints, internal services). This matters for the
/// direct-fetch fallback, which runs from munibot's own network; it is
/// precautionary for the Exa path, where Exa's own infrastructure does the
/// fetching and reaching munibot's internal network was never possible
/// regardless.
///
/// This check has a real, accepted limitation: resolving the host and then
/// letting `reqwest` connect performs DNS twice, and nothing prevents the
/// answer from differing between the two resolutions (DNS rebinding). Closing
/// that gap needs a custom resolver that connects to the exact address already
/// validated, which is future work rather than something this tool silently
/// pretends to have solved.
pub struct WebFetchTool {
    backend: Arc<dyn ExaBackend>,
    client: reqwest::Client,
}

impl WebFetchTool {
    /// Builds the tool over any Exa backend, real or scripted, with its own
    /// direct-fetch client for the fallback path.
    pub fn new(backend: Arc<dyn ExaBackend>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            // never follow a redirect automatically: a validated URL could redirect to an
            // unvalidated one, defeating the whole point of validate_fetch_url
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build reqwest client for web fetch");

        Self { backend, client }
    }

    async fn fetch_directly(&self, url: &Url, max_characters: usize) -> Result<String, String> {
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|error| format!("couldn't reach that url :< {error}"))?;

        if !response.status().is_success() {
            return Err(format!("that url returned {} :<", response.status()));
        }

        let html = response
            .text()
            .await
            .map_err(|error| format!("couldn't read the response body :< {error}"))?;

        extract_text(&html, max_characters)
    }
}

/// Converts raw HTML into plain, wrapped text and truncates it to
/// `max_characters`.
///
/// Separated from [`WebFetchTool::fetch_directly`] so this half of the fallback
/// path - the actual "readability extraction" the plan calls for - is
/// unit-testable directly, without needing a live or mock HTTP server to
/// exercise the network half.
fn extract_text(html: &str, max_characters: usize) -> Result<String, String> {
    let text = html2text::from_read(html.as_bytes(), HTML_WRAP_WIDTH)
        .map_err(|error| format!("couldn't extract text from that page :< {error}"))?;
    Ok(truncate_chars(&text, max_characters))
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetches the content of a specific URL and returns its extracted, readable text. Use this \
         for a URL you already have - from the user, from a prior web_search result, or from a \
         document - rather than to discover new sources."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::NetworkRead
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<WebFetchArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let args: WebFetchArgs = match serde_json::from_value(input) {
            Ok(args) => args,
            Err(error) => return ToolOutcome::err(format!("couldn't parse arguments :< {error}")),
        };
        let max_characters = args.max_characters.unwrap_or(DEFAULT_MAX_CHARACTERS);

        let url = match validate_fetch_url(&args.url).await {
            Ok(url) => url,
            Err(reason) => return ToolOutcome::err(reason),
        };

        let text_options = TextOptions {
            max_characters: Some(max_characters),
        };
        let exa_result = self
            .backend
            .contents(vec![url.to_string()], Some(text_options))
            .await;

        let text = match exa_result {
            Ok(response) => match response
                .results
                .first()
                .and_then(|result| result.text.clone())
            {
                Some(text) if !text.is_empty() => Some(text),
                _ => None,
            },
            Err(_) => None,
        };

        let text = match text {
            Some(text) => text,
            None => match self.fetch_directly(&url, max_characters).await {
                Ok(text) => text,
                Err(reason) => return ToolOutcome::err(reason),
            },
        };

        ToolOutcome::ok(wrap_untrusted("web_fetch", &text))
    }
}

/// Truncates `text` to at most `max_characters` Unicode scalar values, never
/// splitting a multi-byte character.
fn truncate_chars(text: &str, max_characters: usize) -> String {
    text.chars().take(max_characters).collect()
}

/// Rejects anything but `http`/`https` and resolves the host to reject requests
/// aimed at a non-public address range. See the security note on
/// [`WebFetchTool`] for what this does and does not guarantee.
async fn validate_fetch_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("{raw:?} isn't a valid url :< {error}"))?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!(
            "only http and https urls are allowed, not {:?} :<",
            url.scheme()
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| format!("{raw:?} has no host :<"))?;
    let port = url.port_or_known_default().unwrap_or(443);

    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("couldn't resolve {host:?} :< {error}"))?;

    let mut resolved_any = false;
    for address in addresses {
        resolved_any = true;
        if let Some(reason) = reject_reason(address.ip()) {
            return Err(format!(
                "{host:?} resolves to {reason}, which isn't fetchable :<"
            ));
        }
    }

    if !resolved_any {
        return Err(format!("{host:?} didn't resolve to anything :<"));
    }

    Ok(url)
}

/// Names why an address should be rejected, or `None` if it is an ordinary
/// public address.
fn reject_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                Some("a loopback address")
            } else if v4.is_private() {
                Some("a private address")
            } else if v4.is_link_local() {
                Some("a link-local address")
            } else if v4.is_unspecified() {
                Some("an unspecified address")
            } else if v4.is_broadcast() {
                Some("a broadcast address")
            } else if v4.is_documentation() {
                Some("a documentation-reserved address")
            } else if is_carrier_grade_nat(v4.octets()) {
                Some("a carrier-grade NAT address")
            } else {
                None
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                Some("a loopback address")
            } else if v6.is_unicast_link_local() {
                Some("a link-local address")
            } else if v6.is_unique_local() {
                Some("a unique local address")
            } else if v6.is_unspecified() {
                Some("an unspecified address")
            } else if let Some(mapped) = v6.to_ipv4_mapped() {
                // an IPv4-mapped IPv6 address (::ffff:a.b.c.d) must be checked against the same
                // v4 ranges, or this whole check could be bypassed by mapping a private v4
                // address into v6 form
                reject_reason(IpAddr::V4(mapped))
            } else {
                None
            }
        }
    }
}

/// `100.64.0.0/10`, the carrier-grade NAT range (RFC 6598). Checked manually
/// since `Ipv4Addr::is_shared` is not yet stable.
fn is_carrier_grade_nat(octets: [u8; 4]) -> bool {
    octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;
    use crate::tools::{
        ConversationId, Platform,
        exa::{ContentsResponse, ExaResult, FakeExaBackend},
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
            delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    fn tool_with(backend: FakeExaBackend) -> WebFetchTool {
        WebFetchTool::new(Arc::new(backend))
    }

    fn contents_result(url: &str, text: &str) -> ExaResult {
        ExaResult {
            id: url.to_string(),
            title: None,
            url: url.to_string(),
            published_date: None,
            author: None,
            text: Some(text.to_string()),
            highlights: vec![],
            image: None,
            favicon: None,
        }
    }

    // --- reject_reason: the core SSRF gate, exercised directly with no network at
    // all ---

    #[test]
    fn test_rejects_ipv4_loopback() {
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))).is_some());
    }

    #[test]
    fn test_rejects_ipv4_private_ranges() {
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).is_some());
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))).is_some());
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))).is_some());
    }

    #[test]
    fn test_rejects_ipv4_link_local_and_cloud_metadata() {
        // 169.254.169.254 is the AWS/GCP/Azure metadata endpoint - the canonical SSRF
        // target
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))).is_some());
    }

    #[test]
    fn test_rejects_carrier_grade_nat() {
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))).is_some());
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255))).is_some());
    }

    #[test]
    fn test_allows_ordinary_public_ipv4() {
        assert!(reject_reason(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))).is_none());
        assert!(
            reject_reason(IpAddr::V4(Ipv4Addr::new(100, 63, 255, 255))).is_none(),
            "just below the carrier-grade NAT range should be public"
        );
        assert!(
            reject_reason(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 0))).is_none(),
            "just above the carrier-grade NAT range should be public"
        );
    }

    #[test]
    fn test_rejects_ipv6_loopback_and_unique_local() {
        assert!(reject_reason(IpAddr::V6(Ipv6Addr::LOCALHOST)).is_some());
        assert!(reject_reason(IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1))).is_some());
        assert!(reject_reason(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1))).is_some());
    }

    #[test]
    fn test_rejects_ipv4_mapped_private_address_in_ipv6_form() {
        // ::ffff:10.0.0.1 must be caught by the same rule as 10.0.0.1 itself, or
        // wrapping a private address in IPv4-mapped IPv6 form would bypass the
        // whole check
        let mapped = Ipv4Addr::new(10, 0, 0, 1).to_ipv6_mapped();
        assert!(reject_reason(IpAddr::V6(mapped)).is_some());
    }

    #[test]
    fn test_allows_ordinary_public_ipv6() {
        // 2606:4700:4700::1111 is a real public Cloudflare DNS address
        assert!(
            reject_reason(IpAddr::V6(Ipv6Addr::new(
                0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111
            )))
            .is_none()
        );
    }

    // --- validate_fetch_url: scheme and parse rejection, no DNS needed ---

    #[tokio::test]
    async fn test_rejects_non_http_schemes() {
        let result = validate_fetch_url("file:///etc/passwd").await;
        assert!(result.is_err(), "a non-http(s) scheme must be rejected");
    }

    #[tokio::test]
    async fn test_rejects_unparsable_urls() {
        let result = validate_fetch_url("not a url at all").await;
        assert!(result.is_err());
    }

    // --- tool behaviour, all via FakeExaBackend, no network ---

    #[tokio::test]
    async fn test_malformed_arguments_are_a_recoverable_error() {
        let tool = tool_with(FakeExaBackend::respond_contents(Ok(ContentsResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        let outcome = tool.invoke(serde_json::json!({}), &ctx()).await;
        assert!(
            matches!(outcome, ToolOutcome::Err(_)),
            "missing required url should be rejected"
        );
    }

    #[tokio::test]
    async fn test_ssrf_target_is_rejected_before_any_backend_call() {
        // an unused scripted response is harmless here; the assertion below is what
        // actually proves the target was rejected before validate_fetch_url
        // ever returned
        let tool = tool_with(FakeExaBackend::respond_contents(Ok(ContentsResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));

        let outcome = tool
            .invoke(
                serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}),
                &ctx(),
            )
            .await;

        match outcome {
            ToolOutcome::Err(message) => {
                assert!(message.contains("isn't fetchable"), "got {message:?}")
            }
            other => panic!("expected the ssrf target to be rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_exa_contents_result_is_used_when_available() {
        let tool = tool_with(FakeExaBackend::respond_contents(Ok(ContentsResponse {
            request_id: "r1".to_string(),
            results: vec![contents_result(
                "https://example.com/",
                "extracted article text",
            )],
            cost_dollars: None,
        })));

        let outcome = tool
            .invoke(serde_json::json!({"url": "https://example.com/"}), &ctx())
            .await;

        match outcome {
            ToolOutcome::Ok(text) => {
                assert!(text.contains("extracted article text"));
                assert!(text.contains("<untrusted-content"));
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn test_truncate_chars_never_splits_a_multibyte_character() {
        let text = "hello 🌸 world";
        let truncated = truncate_chars(text, 7);
        assert!(
            truncated.chars().count() <= 7,
            "should not exceed the character limit: {truncated:?}"
        );
        assert!(
            std::str::from_utf8(truncated.as_bytes()).is_ok(),
            "truncation must never produce invalid utf-8"
        );
    }

    // --- extract_text: the actual html-to-text "readability extraction" logic, no
    // network ---

    #[test]
    fn test_extract_text_strips_html_tags() {
        let html = "<h1>Title</h1><p>Some <strong>bold</strong> text.</p>";
        let text = extract_text(html, 10_000).expect("should extract");
        assert!(!text.contains('<'), "tags should be stripped: {text:?}");
        assert!(text.contains("Title"));
        assert!(text.contains("bold"));
    }

    #[test]
    fn test_extract_text_truncates_to_max_characters() {
        let html = format!("<p>{}</p>", "a".repeat(1000));
        let text = extract_text(&html, 50).expect("should extract");
        assert!(
            text.chars().count() <= 50,
            "got {} characters",
            text.chars().count()
        );
    }

    #[test]
    fn test_extract_text_handles_empty_html() {
        let text = extract_text("", 100).expect("empty input should not error");
        assert!(text.trim().is_empty());
    }

    #[test]
    fn test_input_schema_requires_only_url() {
        let tool = tool_with(FakeExaBackend::respond_contents(Ok(ContentsResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        let schema = tool.input_schema();
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert_eq!(required, vec![serde_json::json!("url")]);
    }

    #[test]
    fn test_tool_metadata() {
        let tool = tool_with(FakeExaBackend::respond_contents(Ok(ContentsResponse {
            request_id: "r1".to_string(),
            results: vec![],
            cost_dollars: None,
        })));
        assert_eq!(tool.name(), "web_fetch");
        assert_eq!(tool.tier(), RiskTier::NetworkRead);
    }
}
