//! The rpc server: accepts connections on a unix socket, decodes frames into
//! [`ToolRequest`]s, dispatches each by tool name, and writes back a
//! correlated [`ToolResponse`].
//!
//! Every accepted connection is handled concurrently with every other one,
//! and every request *within* one connection is dispatched concurrently with
//! every other request already in flight on it -- the read loop never waits
//! for a dispatch to finish before reading the next frame. A single writer
//! task per connection serializes responses back onto the wire, so two
//! overlapping dispatches finishing out of order can never interleave their
//! frames.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    io::{AsyncWriteExt, WriteHalf},
    net::{UnixListener, UnixStream},
    sync::{mpsc, watch},
    task::JoinSet,
};

use crate::{
    codec::{FrameError, read_frame, write_frame},
    protocol::{ToolRequest, ToolResponse, ToolResult},
};

/// One tool's execution, registered into a [`Dispatcher`] under its name.
///
/// Implemented once per tool as later commits add `read`, `write`, `edit`,
/// `bash`, `grep`, and `glob` -- this commit adds the dispatch mechanism
/// itself, with nothing registered yet.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn call(&self, input: Value) -> ToolResult;
}

/// Routes a decoded [`ToolRequest`] to whichever [`ToolHandler`] is
/// registered under its `tool` name.
#[derive(Default)]
pub struct Dispatcher {
    handlers: HashMap<String, Arc<dyn ToolHandler>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a handler under `name`, replacing whatever was already
    /// there.
    pub fn register(&mut self, name: impl Into<String>, handler: Arc<dyn ToolHandler>) {
        self.handlers.insert(name.into(), handler);
    }

    /// Dispatches one request, producing a response correlated by the same
    /// id -- an unknown tool name is a recoverable [`ToolResult::Err`], not a
    /// connection-ending failure, naming every tool that *is* registered so
    /// whatever is on the other end can see what went wrong.
    pub async fn dispatch(&self, request: ToolRequest) -> ToolResponse {
        let result = match self.handlers.get(&request.tool) {
            Some(handler) => handler.call(request.input).await,
            None => {
                let mut available: Vec<&str> = self.handlers.keys().map(String::as_str).collect();
                available.sort_unstable();
                ToolResult::Err(format!(
                    "no such tool {:?} :< available tools are: {}",
                    request.tool,
                    available.join(", ")
                ))
            }
        };

        ToolResponse {
            id: request.id,
            result,
        }
    }
}

/// Handles one accepted connection until it closes, a framing error ends it,
/// or `shutdown` fires, dispatching every request that arrives on it
/// concurrently.
///
/// On `shutdown`, stops reading *new* requests from this connection
/// immediately - waiting for the peer to close its end first would hang
/// forever against a peer that keeps a long-lived connection open across
/// many calls (exactly how the host's own `RpcClient` uses this) - but still
/// lets every request already dispatched actually finish and its response
/// reach the wire before this connection closes.
async fn handle_connection(
    stream: UnixStream,
    dispatcher: Arc<Dispatcher>,
    mut shutdown: watch::Receiver<()>,
) {
    let (mut read_half, write_half) = tokio::io::split(stream);
    let (tx, rx) = mpsc::channel::<Vec<u8>>(32);

    let writer = tokio::spawn(run_writer(write_half, rx));
    let mut in_flight = JoinSet::new();

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                tracing::debug!("shutdown signalled; draining this connection's in-flight requests");
                break;
            }
            frame = read_frame(&mut read_half) => {
                match frame {
                    Ok(payload) => match serde_json::from_slice::<ToolRequest>(&payload) {
                        Ok(request) => {
                            let dispatcher = Arc::clone(&dispatcher);
                            let tx = tx.clone();
                            in_flight.spawn(async move {
                                let response = dispatcher.dispatch(request).await;
                                if let Ok(encoded) = serde_json::to_vec(&response) {
                                    // the receiver only ever drops once this loop itself
                                    // has already stopped reading, so a failed send here
                                    // just means the connection is already on its way
                                    // down - nothing left to report it to
                                    let _ = tx.send(encoded).await;
                                }
                            });
                        }
                        Err(error) => {
                            tracing::warn!(%error, "received a frame that wasn't a valid ToolRequest");
                            break;
                        }
                    },
                    // the far side closed the connection cleanly - not every request
                    // sent on it needs a response it will never read
                    Err(FrameError::Truncated) => break,
                    Err(error) => {
                        tracing::warn!(%error, "framing error on a connection");
                        break;
                    }
                }
            }
        }
    }

    // stop accepting new work on this connection, then let every dispatch
    // already in flight actually finish before the writer (and the whole
    // connection) shuts down
    drop(tx);
    while in_flight.join_next().await.is_some() {}
    let _ = writer.await;
}

/// Drains encoded [`ToolResponse`] payloads from `rx` and frames each one
/// onto `write_half`, one at a time -- the single point every response for
/// one connection passes through, so concurrent dispatches can never
/// interleave their bytes on the wire.
async fn run_writer(mut write_half: WriteHalf<UnixStream>, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(payload) = rx.recv().await {
        if let Err(error) = write_frame(&mut write_half, &payload).await {
            tracing::warn!(%error, "failed to write a response frame");
            break;
        }
    }
    let _ = write_half.shutdown().await;
}

/// Accepts connections on `listener` until `shutdown` resolves, handling
/// every one concurrently with every other.
///
/// On shutdown, stops accepting new connections immediately but waits for
/// every connection already accepted to finish draining its own in-flight
/// requests (see [`handle_connection`]) before returning - a container being
/// torn down should never cut off a response the model was about to
/// receive for a call it already made.
pub async fn serve(
    listener: UnixListener,
    dispatcher: Arc<Dispatcher>,
    shutdown: impl Future<Output = ()>,
) {
    let mut connections = JoinSet::new();
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => {
                tracing::info!("shutdown signal received; draining in-flight connections");
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let dispatcher = Arc::clone(&dispatcher);
                        connections.spawn(handle_connection(stream, dispatcher, shutdown_rx.clone()));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to accept a connection");
                    }
                }
            }
        }
    }

    // tells every connection already accepted to stop reading new requests
    // and start draining - see handle_connection's own doc comment on why
    // that must happen actively rather than by waiting for the peer to hang
    // up first
    let _ = shutdown_tx.send(());
    while connections.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::net::UnixStream;

    use super::*;

    /// Returns one canned [`ToolResult`] to every call, regardless of input,
    /// and records how many times it was called.
    struct FakeHandler {
        result: ToolResult,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeHandler {
        fn ok(text: &str) -> Arc<Self> {
            Arc::new(Self {
                result: ToolResult::Ok(text.to_string()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl ToolHandler for FakeHandler {
        async fn call(&self, _input: Value) -> ToolResult {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.result.clone()
        }
    }

    /// A handler that waits before responding, so a test can prove a slow
    /// call never blocks a faster one behind it on the same connection.
    struct SlowHandler {
        delay: Duration,
        text: String,
    }

    #[async_trait]
    impl ToolHandler for SlowHandler {
        async fn call(&self, _input: Value) -> ToolResult {
            tokio::time::sleep(self.delay).await;
            ToolResult::Ok(self.text.clone())
        }
    }

    fn socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "munibot_toolagent_test_{name}_{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn test_dispatch_routes_to_the_registered_handler_by_name() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.register("read", FakeHandler::ok("file contents"));

        let response = dispatcher
            .dispatch(ToolRequest {
                id: 1,
                tool: "read".to_string(),
                input: json!({}),
            })
            .await;

        assert_eq!(response.id, 1);
        assert_eq!(response.result, ToolResult::Ok("file contents".to_string()));
    }

    #[tokio::test]
    async fn test_dispatch_names_available_tools_for_an_unknown_one() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.register("read", FakeHandler::ok("unused"));
        dispatcher.register("write", FakeHandler::ok("unused"));

        let response = dispatcher
            .dispatch(ToolRequest {
                id: 2,
                tool: "delete_everything".to_string(),
                input: json!({}),
            })
            .await;

        assert_eq!(response.id, 2);
        match response.result {
            ToolResult::Err(message) => {
                assert!(message.contains("delete_everything"));
                assert!(message.contains("read"));
                assert!(message.contains("write"));
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_preserves_the_request_id() {
        let mut dispatcher = Dispatcher::new();
        dispatcher.register("read", FakeHandler::ok("x"));

        let response = dispatcher
            .dispatch(ToolRequest {
                id: 42,
                tool: "read".to_string(),
                input: json!({}),
            })
            .await;

        assert_eq!(response.id, 42);
    }

    #[tokio::test]
    async fn test_a_client_gets_a_correlated_response_over_the_socket() {
        let path = socket_path("basic");
        let listener = UnixListener::bind(&path).expect("should bind");

        let mut dispatcher = Dispatcher::new();
        dispatcher.register("read", FakeHandler::ok("hello from the sandbox"));
        let dispatcher = Arc::new(dispatcher);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve(listener, dispatcher, async {
            let _ = shutdown_rx.await;
        }));

        let mut client = UnixStream::connect(&path).await.expect("should connect");
        let request = ToolRequest {
            id: 1,
            tool: "read".to_string(),
            input: json!({"path": "src/main.rs"}),
        };
        write_frame(&mut client, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();

        let payload = read_frame(&mut client)
            .await
            .expect("should read a response");
        let response: ToolResponse = serde_json::from_slice(&payload).unwrap();

        assert_eq!(response.id, 1);
        assert_eq!(
            response.result,
            ToolResult::Ok("hello from the sandbox".to_string())
        );

        let _ = shutdown_tx.send(());
        server.await.unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_a_fast_request_is_not_blocked_behind_a_slow_one_on_the_same_connection() {
        let path = socket_path("concurrent");
        let listener = UnixListener::bind(&path).expect("should bind");

        let mut dispatcher = Dispatcher::new();
        dispatcher.register(
            "slow",
            Arc::new(SlowHandler {
                delay: Duration::from_millis(200),
                text: "slow done".to_string(),
            }),
        );
        dispatcher.register("fast", FakeHandler::ok("fast done"));
        let dispatcher = Arc::new(dispatcher);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve(listener, dispatcher, async {
            let _ = shutdown_rx.await;
        }));

        let mut client = UnixStream::connect(&path).await.expect("should connect");

        // the slow request is sent first, but the read loop must move on to
        // read (and dispatch) the fast one immediately rather than waiting
        write_frame(
            &mut client,
            &serde_json::to_vec(&ToolRequest {
                id: 1,
                tool: "slow".to_string(),
                input: json!({}),
            })
            .unwrap(),
        )
        .await
        .unwrap();
        write_frame(
            &mut client,
            &serde_json::to_vec(&ToolRequest {
                id: 2,
                tool: "fast".to_string(),
                input: json!({}),
            })
            .unwrap(),
        )
        .await
        .unwrap();

        // the fast response should arrive well before the slow handler's
        // 200ms delay elapses, proving it wasn't queued behind it
        let first_response: ToolResponse =
            tokio::time::timeout(Duration::from_millis(100), async {
                let payload = read_frame(&mut client).await.unwrap();
                serde_json::from_slice(&payload).unwrap()
            })
            .await
            .expect("the fast response should arrive quickly, not after the slow one's delay");

        assert_eq!(first_response.id, 2, "the fast call should respond first");

        let second_response: ToolResponse = {
            let payload = read_frame(&mut client).await.unwrap();
            serde_json::from_slice(&payload).unwrap()
        };
        assert_eq!(second_response.id, 1);

        let _ = shutdown_tx.send(());
        server.await.unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn test_shutdown_stops_accepting_but_drains_in_flight_connections() {
        let path = socket_path("shutdown");
        let listener = UnixListener::bind(&path).expect("should bind");

        let mut dispatcher = Dispatcher::new();
        dispatcher.register(
            "slow",
            Arc::new(SlowHandler {
                delay: Duration::from_millis(150),
                text: "finished despite shutdown".to_string(),
            }),
        );
        let dispatcher = Arc::new(dispatcher);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve(listener, dispatcher, async {
            let _ = shutdown_rx.await;
        }));

        let mut client = UnixStream::connect(&path).await.expect("should connect");
        write_frame(
            &mut client,
            &serde_json::to_vec(&ToolRequest {
                id: 1,
                tool: "slow".to_string(),
                input: json!({}),
            })
            .unwrap(),
        )
        .await
        .unwrap();
        // gives the server a moment to actually accept the connection and
        // dispatch the slow call before shutdown fires, so this test
        // exercises "a call already in flight when shutdown arrives" rather
        // than racing whether the connection was accepted at all
        tokio::time::sleep(Duration::from_millis(20)).await;

        // signal shutdown while the slow call is still running
        let _ = shutdown_tx.send(());

        // the in-flight call must still complete and be readable, even
        // though shutdown was already signalled before it finished
        let payload = tokio::time::timeout(Duration::from_millis(500), read_frame(&mut client))
            .await
            .expect("should not hang")
            .expect("should read a response");
        let response: ToolResponse = serde_json::from_slice(&payload).unwrap();
        assert_eq!(
            response.result,
            ToolResult::Ok("finished despite shutdown".to_string())
        );

        server.await.unwrap();
        std::fs::remove_file(&path).ok();
    }
}
