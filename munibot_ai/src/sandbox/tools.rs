//! The six sandboxed `Tool` implementations - `read`, `write`, `edit`,
//! `bash`, `grep`, `glob` - each marshalling its call to a running
//! sandbox's [`RpcClient`] and back.
//!
//! Every implementation here holds no logic beyond argument marshalling and
//! the read-before-write guard `write` enforces; all real execution happens
//! in `munibot_toolagent`, on the other side of the connection.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde_json::Value;

use crate::{
    sandbox::{protocol::ToolResult, rpc::RpcClient},
    tools::{RiskTier, Tool, ToolCtx, ToolOutcome, wrap_untrusted},
    types::ToolSchema,
};

/// Reads `input["path"]` as a string, for the two tools ([`ReadTool`],
/// [`WriteTool`]) that need it outside of the arguments they otherwise pass
/// straight through unexamined - a malformed or missing path is left for
/// `munibot_toolagent`'s own argument validation to reject with a proper
/// error; this only degrades to an empty string so the read-tracking
/// bookkeeping has *something* to key on rather than panicking.
fn path_argument(input: &Value) -> String {
    input
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// State shared by every sandboxed tool wired to one running sandbox:
/// exactly the connection, plus which repository-relative paths `read` has
/// already surfaced to the model this session.
///
/// [`WriteTool`] consults this before overwriting a path it has never seen
/// read - the classic failure this guards against is a model overwriting a
/// file it never actually looked at, guessing at content that was already
/// there. A fresh path (nothing there yet) is always allowed to be created;
/// only overwriting something already on disk requires having read it
/// first.
pub struct SandboxSession {
    client: Arc<RpcClient>,
    read_paths: Mutex<HashSet<String>>,
}

impl SandboxSession {
    pub fn new(client: Arc<RpcClient>) -> Arc<Self> {
        Arc::new(Self {
            client,
            read_paths: Mutex::new(HashSet::new()),
        })
    }

    fn mark_read(&self, path: &str) {
        self.read_paths.lock().unwrap().insert(path.to_string());
    }

    fn has_read(&self, path: &str) -> bool {
        self.read_paths.lock().unwrap().contains(path)
    }
}

/// Calls `tool` on `session`'s connection with `input`, converting the
/// result into a [`ToolOutcome`] - any failure at the rpc layer itself
/// (a timeout, a cancelled call, a dead connection) becomes `Fatal`, the
/// same reasoning [`ToolOutcome::Fatal`]'s own doc comment gives for a
/// cancelled context or a provider outage: nothing the model does
/// differently on its next call can fix a broken sandbox connection.
async fn call(
    session: &SandboxSession,
    tool: &str,
    input: Value,
    timeout: Duration,
    ctx: &ToolCtx,
) -> Result<String, ToolOutcome> {
    match session
        .client
        .call(tool, input, timeout, &ctx.cancellation)
        .await
    {
        Ok(ToolResult::Ok(text)) => Ok(text),
        Ok(ToolResult::Err(message)) => Err(ToolOutcome::err(message)),
        Err(error) => Err(ToolOutcome::fatal(error)),
    }
}

const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const EDIT_TIMEOUT: Duration = Duration::from_secs(30);
const GREP_TIMEOUT: Duration = Duration::from_secs(60);
const GLOB_TIMEOUT: Duration = Duration::from_secs(60);
// generous over munibot_toolagent's own MAX_TIMEOUT_SECS (600s) so this
// call's own timeout is never what cuts a long-running command off - the
// remote timeout the model itself asked for should always win that race
const BASH_TIMEOUT: Duration = Duration::from_secs(630);

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ReadArgs {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub struct ReadTool {
    session: Arc<SandboxSession>,
}

impl ReadTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Reads a file from the checked-out repository, returning its content with line numbers. \
         Read a file before editing or overwriting it."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<ReadArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        let path = path_argument(&input);

        match call(&self.session, self.name(), input, READ_TIMEOUT, ctx).await {
            Ok(text) => {
                self.session.mark_read(&path);
                ToolOutcome::ok(wrap_untrusted(self.name(), &text))
            }
            Err(outcome) => outcome,
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct WriteArgs {
    path: String,
    content: String,
}

/// Overwrites a file the session has not read are refused - see
/// [`SandboxSession`]'s own doc comment.
pub struct WriteTool {
    session: Arc<SandboxSession>,
}

impl WriteTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }

    /// Whether `path` already exists in the repository, probed with a
    /// throwaway `read` call rather than a dedicated existence check the
    /// agent protocol has no verb for.
    async fn exists(&self, path: &str, ctx: &ToolCtx) -> bool {
        let probe = serde_json::json!({"path": path, "limit": 1});
        matches!(
            self.session
                .client
                .call("read", probe, READ_TIMEOUT, &ctx.cancellation)
                .await,
            Ok(ToolResult::Ok(_))
        )
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Creates or overwrites a file in the checked-out repository. Overwriting an existing file \
         requires having read it first this session - read it before writing if you haven't \
         already."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<WriteArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        let path = path_argument(&input);

        if !self.session.has_read(&path) && self.exists(&path, ctx).await {
            return ToolOutcome::err(format!(
                "{path:?} already exists and hasn't been read this session :< read it first so \
                 you don't overwrite something you haven't looked at"
            ));
        }

        match call(&self.session, self.name(), input, WRITE_TIMEOUT, ctx).await {
            Ok(text) => {
                self.session.mark_read(&path);
                ToolOutcome::ok(text)
            }
            Err(outcome) => outcome,
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

pub struct EditTool {
    session: Arc<SandboxSession>,
}

impl EditTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replaces an exact string in a file in the checked-out repository. Fails rather than \
         guessing when old_string is absent or appears more than once, unless replace_all is set."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<EditArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        match call(&self.session, self.name(), input, EDIT_TIMEOUT, ctx).await {
            Ok(text) => ToolOutcome::ok(text),
            Err(outcome) => outcome,
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct GrepArgs {
    pattern: String,
    include: Option<String>,
}

pub struct GrepTool {
    session: Arc<SandboxSession>,
}

impl GrepTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Searches the checked-out repository for a regular expression, returning matching lines \
         with file paths and line numbers. Optionally narrow to files matching an include glob."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<GrepArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        match call(&self.session, self.name(), input, GREP_TIMEOUT, ctx).await {
            Ok(text) => ToolOutcome::ok(wrap_untrusted(self.name(), &text)),
            Err(outcome) => outcome,
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct GlobArgs {
    pattern: String,
}

pub struct GlobTool {
    session: Arc<SandboxSession>,
}

impl GlobTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Finds files in the checked-out repository matching a glob pattern (e.g. \"**/*.rs\"), \
         respecting .gitignore, most recently modified first."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<GlobArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        match call(&self.session, self.name(), input, GLOB_TIMEOUT, ctx).await {
            Ok(text) => ToolOutcome::ok(wrap_untrusted(self.name(), &text)),
            Err(outcome) => outcome,
        }
    }
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct BashArgs {
    command: String,
    working_directory: Option<String>,
    timeout_secs: Option<u64>,
}

/// Marked [`Tool::is_serial`] - a persistent shell session inside one
/// sandbox is exactly the motivating case that method's own doc comment
/// describes: two calls to it in the same batch must never run at the same
/// time.
pub struct BashTool {
    session: Arc<SandboxSession>,
}

impl BashTool {
    pub fn new(session: Arc<SandboxSession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Runs a shell command in the checked-out repository, with an optional working directory \
         and timeout. Output over a few thousand bytes per stream is paged to a file readable with \
         read."
    }

    fn tier(&self) -> RiskTier {
        RiskTier::Sandbox
    }

    fn is_serial(&self) -> bool {
        true
    }

    fn input_schema(&self) -> Value {
        ToolSchema::from_schemars::<BashArgs>(self.name(), self.description()).input_schema
    }

    async fn invoke(&self, input: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(error) = ctx.require_tier(self.tier()) {
            return ToolOutcome::fatal(error);
        }

        match call(&self.session, self.name(), input, BASH_TIMEOUT, ctx).await {
            Ok(text) => ToolOutcome::ok(wrap_untrusted(self.name(), &text)),
            Err(outcome) => outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::net::UnixListener;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        harness::Budget,
        sandbox::{codec::write_frame, protocol::ToolRequest},
        tools::{ConversationId, Platform},
    };

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Web,
            granted_tier,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: Budget::default(),
            delegation_spend: Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    /// A scripted fake tool agent: replies to every request according to
    /// `respond`, so these tests exercise real marshalling and the
    /// read-before-write guard without a real sandbox.
    async fn fake_agent<F>(listener: UnixListener, respond: F)
    where
        F: Fn(&ToolRequest) -> ToolResult + Send + Sync + 'static,
    {
        let (stream, _) = listener.accept().await.expect("should accept");
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        while let Ok(payload) = crate::sandbox::codec::read_frame(&mut read_half).await {
            let request: ToolRequest = serde_json::from_slice(&payload).unwrap();
            let response = crate::sandbox::protocol::ToolResponse {
                id: request.id,
                result: respond(&request),
            };
            let encoded = serde_json::to_vec(&response).unwrap();
            if write_frame(&mut write_half, &encoded).await.is_err() {
                break;
            }
        }
    }

    fn socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "munibot_ai_sandbox_tools_test_{name}_{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn session_with<F>(name: &str, respond: F) -> Arc<SandboxSession>
    where
        F: Fn(&ToolRequest) -> ToolResult + Send + Sync + 'static,
    {
        let path = socket_path(name);
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(fake_agent(listener, respond));

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");
        SandboxSession::new(Arc::new(client))
    }

    #[tokio::test]
    async fn test_read_tool_marks_the_path_as_read_on_success() {
        let session = session_with("read_marks", |_| {
            ToolResult::Ok("1: fn main() {}".to_string())
        })
        .await;
        assert!(!session.has_read("src/main.rs"));

        let tool = ReadTool::new(Arc::clone(&session));
        let outcome = tool
            .invoke(json!({"path": "src/main.rs"}), &ctx(RiskTier::Sandbox))
            .await;

        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("fn main")),
            other => panic!("expected success, got {other:?}"),
        }
        assert!(session.has_read("src/main.rs"));
    }

    #[tokio::test]
    async fn test_read_tool_wraps_output_as_untrusted() {
        let session =
            session_with("read_untrusted", |_| ToolResult::Ok("contents".to_string())).await;
        let tool = ReadTool::new(session);
        let outcome = tool
            .invoke(json!({"path": "f.rs"}), &ctx(RiskTier::Sandbox))
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("<untrusted-content")),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_read_tool_requires_sandbox_tier() {
        let session = session_with("read_tier", |_| ToolResult::Ok("x".to_string())).await;
        let tool = ReadTool::new(session);
        let outcome = tool
            .invoke(json!({"path": "f.rs"}), &ctx(RiskTier::Safe))
            .await;
        assert!(matches!(outcome, ToolOutcome::Fatal(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_write_tool_allows_creating_a_brand_new_file() {
        // the fake agent's read probe always fails ("does not exist"),
        // simulating a genuinely new path
        let session = session_with("write_new_file", |request| {
            if request.tool == "read" {
                ToolResult::Err("not found".to_string())
            } else {
                ToolResult::Ok("wrote 5 bytes".to_string())
            }
        })
        .await;

        let tool = WriteTool::new(session);
        let outcome = tool
            .invoke(
                json!({"path": "new.rs", "content": "hello"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;

        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_write_tool_refuses_an_existing_file_that_was_never_read() {
        // the fake agent's read probe always succeeds, simulating a file
        // that is already there
        let session = session_with("write_unread_existing", |request| {
            match request.tool.as_str() {
                "read" => ToolResult::Ok("1: old content".to_string()),
                _ => ToolResult::Ok("wrote 5 bytes".to_string()),
            }
        })
        .await;

        let tool = WriteTool::new(session);
        let outcome = tool
            .invoke(
                json!({"path": "existing.rs", "content": "new content"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;

        match outcome {
            ToolOutcome::Err(message) => assert!(message.contains("hasn't been read")),
            other => panic!("expected a recoverable error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_write_tool_allows_an_existing_file_that_was_already_read() {
        let session = session_with("write_read_first", |request| match request.tool.as_str() {
            "read" => ToolResult::Ok("1: old content".to_string()),
            _ => ToolResult::Ok("wrote 5 bytes".to_string()),
        })
        .await;

        // read it first, exactly as the tool's own description asks
        let read_tool = ReadTool::new(Arc::clone(&session));
        read_tool
            .invoke(json!({"path": "existing.rs"}), &ctx(RiskTier::Sandbox))
            .await;

        let write_tool = WriteTool::new(session);
        let outcome = write_tool
            .invoke(
                json!({"path": "existing.rs", "content": "new content"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;

        assert!(matches!(outcome, ToolOutcome::Ok(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_write_tool_marks_the_path_as_read_after_a_successful_write() {
        let session = session_with("write_marks_read", |request| match request.tool.as_str() {
            "read" => ToolResult::Err("not found".to_string()),
            _ => ToolResult::Ok("wrote 5 bytes".to_string()),
        })
        .await;

        let write_tool = WriteTool::new(Arc::clone(&session));
        write_tool
            .invoke(
                json!({"path": "brand_new.rs", "content": "hi"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;

        assert!(
            session.has_read("brand_new.rs"),
            "a file just written should count as read for any later overwrite this session"
        );
    }

    #[tokio::test]
    async fn test_edit_tool_passes_through_the_agents_result() {
        let session = session_with("edit_passthrough", |_| {
            ToolResult::Ok("replaced 1 occurrence".to_string())
        })
        .await;
        let tool = EditTool::new(session);
        let outcome = tool
            .invoke(
                json!({"path": "f.rs", "old_string": "a", "new_string": "b"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert_eq!(text, "replaced 1 occurrence"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_edit_tool_surfaces_a_recoverable_agent_error() {
        let session = session_with("edit_error", |_| {
            ToolResult::Err("old_string wasn't found".to_string())
        })
        .await;
        let tool = EditTool::new(session);
        let outcome = tool
            .invoke(
                json!({"path": "f.rs", "old_string": "a", "new_string": "b"}),
                &ctx(RiskTier::Sandbox),
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Err(_)), "got {outcome:?}");
    }

    #[tokio::test]
    async fn test_grep_tool_wraps_output_as_untrusted() {
        let session = session_with("grep_untrusted", |_| {
            ToolResult::Ok("src/main.rs:1: fn main".to_string())
        })
        .await;
        let tool = GrepTool::new(session);
        let outcome = tool
            .invoke(json!({"pattern": "fn main"}), &ctx(RiskTier::Sandbox))
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("<untrusted-content")),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_glob_tool_wraps_output_as_untrusted() {
        let session = session_with("glob_untrusted", |_| {
            ToolResult::Ok("src/main.rs".to_string())
        })
        .await;
        let tool = GlobTool::new(session);
        let outcome = tool
            .invoke(json!({"pattern": "**/*.rs"}), &ctx(RiskTier::Sandbox))
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("<untrusted-content")),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_tool_is_serial() {
        let session = session_with("bash_serial", |_| {
            ToolResult::Ok("exit code: 0".to_string())
        })
        .await;
        let tool = BashTool::new(session);
        assert!(
            tool.is_serial(),
            "a shared shell session must never race itself"
        );
    }

    #[tokio::test]
    async fn test_bash_tool_wraps_output_as_untrusted() {
        let session = session_with("bash_untrusted", |_| {
            ToolResult::Ok("exit code: 0\nstdout:\nhi".to_string())
        })
        .await;
        let tool = BashTool::new(session);
        let outcome = tool
            .invoke(json!({"command": "echo hi"}), &ctx(RiskTier::Sandbox))
            .await;
        match outcome {
            ToolOutcome::Ok(text) => assert!(text.contains("<untrusted-content")),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_disconnected_session_is_a_fatal_outcome_not_recoverable() {
        let path = socket_path("broken_connection");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            // accepts, then immediately drops the connection with no reply
            let (_stream, _) = listener.accept().await.expect("should accept");
        });

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");
        let session = SandboxSession::new(Arc::new(client));
        let tool = ReadTool::new(session);

        let outcome = tool
            .invoke(json!({"path": "f.rs"}), &ctx(RiskTier::Sandbox))
            .await;
        assert!(
            matches!(outcome, ToolOutcome::Fatal(_)),
            "a broken sandbox connection should never be presented as something the model can fix \
             by retrying: got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn test_every_sandbox_tool_is_tier_sandbox() {
        let session = session_with("tiers", |_| ToolResult::Ok("x".to_string())).await;
        assert_eq!(
            ReadTool::new(Arc::clone(&session)).tier(),
            RiskTier::Sandbox
        );
        assert_eq!(
            WriteTool::new(Arc::clone(&session)).tier(),
            RiskTier::Sandbox
        );
        assert_eq!(
            EditTool::new(Arc::clone(&session)).tier(),
            RiskTier::Sandbox
        );
        assert_eq!(
            GrepTool::new(Arc::clone(&session)).tier(),
            RiskTier::Sandbox
        );
        assert_eq!(
            GlobTool::new(Arc::clone(&session)).tier(),
            RiskTier::Sandbox
        );
        assert_eq!(BashTool::new(session).tier(), RiskTier::Sandbox);
    }
}
