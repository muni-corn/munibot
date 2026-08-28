//! The host-side rpc client: connects to a sandbox's tool agent over its
//! unix socket, and dispatches correlated, concurrent, cancellable tool
//! calls over that one connection.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use tokio::{
    io::{AsyncWriteExt, ReadHalf, WriteHalf},
    net::UnixStream,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    sandbox::{
        codec::{read_frame, write_frame},
        container::Sandbox,
        protocol::{ToolRequest, ToolResponse, ToolResult},
    },
    types::AiError,
};

/// How often [`RpcClient::connect`] retries while waiting for the tool
/// agent's socket to become connectable.
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(50);

impl Sandbox {
    /// Connects to this sandbox's tool agent, waiting up to `timeout` for
    /// its socket to become connectable - the container needs a moment
    /// after `start()` to actually bind it.
    pub async fn connect_tool_agent(&self, timeout: Duration) -> Result<RpcClient, AiError> {
        RpcClient::connect(&self.socket_host_path(), timeout).await
    }
}

/// A live connection to one sandbox's tool agent.
///
/// Every call shares this one connection - a background reader task
/// correlates each incoming [`ToolResponse`] by id back to the [`Self::call`]
/// awaiting it, so several calls can be in flight over the same socket at
/// once, the same way the agent's own server dispatches them concurrently
/// (see `munibot_toolagent::server`).
pub struct RpcClient {
    writer: mpsc::Sender<Vec<u8>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ToolResponse>>>>,
    next_id: AtomicU64,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

impl std::fmt::Debug for RpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcClient")
            .field(
                "pending",
                &self
                    .pending
                    .lock()
                    .map(|pending| pending.len())
                    .unwrap_or(0),
            )
            .finish()
    }
}

impl RpcClient {
    /// Connects to `socket_path`, retrying at [`CONNECT_RETRY_INTERVAL`]
    /// until either it succeeds or `timeout` elapses.
    pub async fn connect(socket_path: &Path, timeout: Duration) -> Result<Self, AiError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match UnixStream::connect(socket_path).await {
                Ok(stream) => return Ok(Self::from_stream(stream)),
                Err(error) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(AiError::Other(format!(
                            "couldn't connect to the sandbox's tool agent within {timeout:?} :< \
                             {error}"
                        )));
                    }
                    tokio::time::sleep(CONNECT_RETRY_INTERVAL).await;
                }
            }
        }
    }

    fn from_stream(stream: UnixStream) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = mpsc::channel(32);

        let writer_task = tokio::spawn(run_writer(write_half, rx));
        let reader_task = tokio::spawn(run_reader(read_half, Arc::clone(&pending)));

        Self {
            writer: tx,
            pending,
            next_id: AtomicU64::new(1),
            reader_task,
            writer_task,
        }
    }

    /// Calls `tool` with `input`, racing the response against `timeout` and
    /// `cancellation` - whichever loses, the pending entry is removed so a
    /// response that eventually does arrive late has nothing left to
    /// deliver it to.
    pub async fn call(
        &self,
        tool: impl Into<String>,
        input: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<ToolResult, AiError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, response_tx);

        let request = ToolRequest {
            id,
            tool: tool.into(),
            input,
        };
        let encoded = serde_json::to_vec(&request)
            .map_err(|error| AiError::Other(format!("couldn't encode a tool call :< {error}")))?;

        if self.writer.send(encoded).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(AiError::Other(
                "the sandbox's tool agent connection is closed".to_string(),
            ));
        }

        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                self.pending.lock().unwrap().remove(&id);
                Err(AiError::Cancelled)
            }
            result = tokio::time::timeout(timeout, response_rx) => match result {
                Ok(Ok(response)) => Ok(response.result),
                Ok(Err(_)) => Err(AiError::Other(
                    "the sandbox's tool agent connection closed before responding".to_string(),
                )),
                Err(_) => {
                    self.pending.lock().unwrap().remove(&id);
                    Err(AiError::Other(format!("tool call timed out after {timeout:?}")))
                }
            },
        }
    }
}

impl Drop for RpcClient {
    /// Neither background task holds anything worth finishing gracefully
    /// once nothing can observe their results any more - the pending map
    /// they'd otherwise keep operating on has already gone with `self`.
    fn drop(&mut self) {
        self.reader_task.abort();
        self.writer_task.abort();
    }
}

/// Drains encoded [`ToolRequest`] payloads from `rx` and frames each one
/// onto `write_half`, one at a time.
async fn run_writer(mut write_half: WriteHalf<UnixStream>, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(payload) = rx.recv().await {
        if let Err(error) = write_frame(&mut write_half, &payload).await {
            tracing::warn!(%error, "failed to write a tool call frame");
            break;
        }
    }
    let _ = write_half.shutdown().await;
}

/// Reads [`ToolResponse`] frames from `read_half` and delivers each one to
/// whichever [`RpcClient::call`] is waiting on its id, until the connection
/// closes or a framing error ends it - at which point every still-pending
/// call is left to discover the connection is gone on its own, by its
/// sender being dropped out from under it.
async fn run_reader(
    mut read_half: ReadHalf<UnixStream>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<ToolResponse>>>>,
) {
    while let Ok(payload) = read_frame(&mut read_half).await {
        match serde_json::from_slice::<ToolResponse>(&payload) {
            Ok(response) => {
                if let Some(sender) = pending.lock().unwrap().remove(&response.id) {
                    // the receiver only stops listening once its own
                    // call() has already given up (timed out or was
                    // cancelled), so a failed send here just means the
                    // response arrived too late to matter
                    let _ = sender.send(response);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "received a frame that wasn't a valid ToolResponse");
            }
        }
    }

    pending.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::net::UnixListener;

    use super::*;

    fn socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "munibot_ai_rpc_test_{name}_{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Runs a minimal fake tool agent - accepts one connection, and for
    /// every request, replies with `ToolResult::Ok("<tool>:<input>")` after
    /// an optional artificial delay, so tests can exercise real
    /// correlation and concurrency without a real container.
    async fn fake_agent(listener: UnixListener, delay: Duration) {
        let (stream, _) = listener.accept().await.expect("should accept");
        let (mut read_half, write_half) = tokio::io::split(stream);
        let write_half = Arc::new(tokio::sync::Mutex::new(write_half));

        loop {
            let payload = match read_frame(&mut read_half).await {
                Ok(payload) => payload,
                Err(_) => break,
            };
            let request: ToolRequest = serde_json::from_slice(&payload).unwrap();
            let write_half = Arc::clone(&write_half);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let response = ToolResponse {
                    id: request.id,
                    result: ToolResult::Ok(format!("{}:{}", request.tool, request.input)),
                };
                let encoded = serde_json::to_vec(&response).unwrap();
                let mut write_half = write_half.lock().await;
                let _ = write_frame(&mut *write_half, &encoded).await;
            });
        }
    }

    #[tokio::test]
    async fn test_call_returns_the_correlated_response() {
        let path = socket_path("basic");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(fake_agent(listener, Duration::ZERO));

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");

        let result = client
            .call(
                "read",
                json!("hi"),
                Duration::from_secs(2),
                &CancellationToken::new(),
            )
            .await
            .expect("should succeed");
        assert_eq!(result, ToolResult::Ok("read:\"hi\"".to_string()));

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_concurrent_calls_do_not_cross_correlate() {
        let path = socket_path("concurrent");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(fake_agent(listener, Duration::from_millis(20)));

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");

        let cancellation = CancellationToken::new();
        let (a, b, c) = tokio::join!(
            client.call("a", json!(1), Duration::from_secs(2), &cancellation),
            client.call("b", json!(2), Duration::from_secs(2), &cancellation),
            client.call("c", json!(3), Duration::from_secs(2), &cancellation),
        );

        assert_eq!(a.unwrap(), ToolResult::Ok("a:1".to_string()));
        assert_eq!(b.unwrap(), ToolResult::Ok("b:2".to_string()));
        assert_eq!(c.unwrap(), ToolResult::Ok("c:3".to_string()));

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_call_times_out_when_the_agent_never_responds() {
        let path = socket_path("timeout");
        let listener = UnixListener::bind(&path).unwrap();
        // never actually reads/responds - just accepts and does nothing
        tokio::spawn(async move {
            let _stream = listener.accept().await;
            std::future::pending::<()>().await;
        });

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");

        let error = client
            .call(
                "slow",
                json!({}),
                Duration::from_millis(100),
                &CancellationToken::new(),
            )
            .await
            .expect_err("should time out");
        assert!(error.to_string().contains("timed out"));

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_call_is_cancelled_by_the_cancellation_token() {
        let path = socket_path("cancel");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let _stream = listener.accept().await;
            std::future::pending::<()>().await;
        });

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");

        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = client
            .call("slow", json!({}), Duration::from_secs(30), &cancellation)
            .await
            .expect_err("should be cancelled");
        assert!(matches!(error, AiError::Cancelled), "got {error:?}");

        std::fs::remove_file(&path).ok();
    }

    /// Spawns the **real** `munibot_toolagent` binary (via `cargo run -p`,
    /// so it is built if it is not already) and talks to it over this
    /// module's own `RpcClient` - the one place the two independently
    /// hand-mirrored wire protocol implementations (this crate's
    /// `sandbox::protocol`/`sandbox::codec`, and `munibot_toolagent`'s own)
    /// are checked against each other for real, rather than each just
    /// testing itself.
    #[tokio::test]
    #[cfg_attr(not(feature = "sandbox-integration"), ignore)]
    async fn test_talks_to_the_real_tool_agent_binary() {
        let root = std::env::temp_dir().join(format!(
            "munibot_ai_rpc_real_agent_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello.txt"), "hello from a real tool agent").unwrap();

        let path = socket_path("real_agent");
        let mut child = tokio::process::Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "-p",
                "munibot_toolagent",
                "--",
                "--socket",
                &path.display().to_string(),
                "--root",
                &root.display().to_string(),
            ])
            .kill_on_drop(true)
            .spawn()
            .expect("should spawn cargo run");

        // generous: cargo may need to build the binary first
        let client = RpcClient::connect(&path, Duration::from_secs(60))
            .await
            .expect("should connect to the real tool agent");

        let result = client
            .call(
                "read",
                json!({"path": "hello.txt"}),
                Duration::from_secs(5),
                &CancellationToken::new(),
            )
            .await
            .expect("should succeed");
        assert_eq!(
            result,
            ToolResult::Ok("1: hello from a real tool agent".to_string())
        );

        let _ = child.kill().await;
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_connect_times_out_when_nothing_is_listening() {
        let path = socket_path("no_listener");
        let error = RpcClient::connect(&path, Duration::from_millis(150))
            .await
            .expect_err("should time out");
        assert!(error.to_string().contains("couldn't connect"));
    }

    #[tokio::test]
    async fn test_connect_succeeds_once_the_listener_appears_after_a_delay() {
        let path = socket_path("delayed_listener");
        let path_clone = path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let listener = UnixListener::bind(&path_clone).unwrap();
            fake_agent(listener, Duration::ZERO).await;
        });

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should eventually connect");
        let result = client
            .call(
                "ping",
                json!(null),
                Duration::from_secs(2),
                &CancellationToken::new(),
            )
            .await
            .expect("should succeed");
        assert_eq!(result, ToolResult::Ok("ping:null".to_string()));

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_a_pending_call_reports_the_connection_closed_if_the_agent_disconnects() {
        let path = socket_path("disconnect");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            // accepts, then drops the connection without ever responding
            // to whatever the client sends - the disconnect itself is what
            // this test exercises, not a reply
            let (_stream, _) = listener.accept().await.expect("should accept");
        });

        let client = RpcClient::connect(&path, Duration::from_secs(2))
            .await
            .expect("should connect");

        // a long timeout, so the timeout branch can never race ahead of
        // the disconnect this test means to observe
        let error = client
            .call(
                "read",
                json!({}),
                Duration::from_secs(5),
                &CancellationToken::new(),
            )
            .await
            .expect_err("should fail once the connection closes with no response");
        assert!(error.to_string().contains("closed"), "got {error}");

        std::fs::remove_file(&path).ok();
    }
}
