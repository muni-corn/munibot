//! The in-container rpc server executing filesystem and shell tools for the
//! ai sandbox (`munibot_ai::sandbox`).
//!
//! Baked into the sandbox container image and started once per sandbox by
//! the host, over a unix socket mounted from a host tmpfs directory. Takes
//! no munibot dependency at all -- see this crate's `Cargo.toml` for why.

use std::sync::Arc;

use clap::Parser;
use munibot_toolagent::server::{Dispatcher, serve};
use tokio::net::UnixListener;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
struct Args {
    /// Path to the unix socket to listen on, mounted into the container by
    /// the host.
    #[clap(long)]
    socket: String,
}

/// Resolves once `SIGTERM` arrives, or immediately if this process somehow
/// cannot install the handler at all (logging why, rather than running with
/// no way to ever shut down gracefully).
async fn wait_for_sigterm() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(error) => {
            tracing::error!(%error, "couldn't install a SIGTERM handler");
        }
    }
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();

    let args = Args::parse();

    // a socket file left behind by a previous, uncleanly-stopped run would
    // otherwise make every fresh bind fail with AddrInUse
    std::fs::remove_file(&args.socket).ok();

    let listener = match UnixListener::bind(&args.socket) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(socket = %args.socket, %error, "couldn't bind the tool agent socket");
            std::process::exit(1);
        }
    };

    tracing::info!(socket = %args.socket, "starting munibot_toolagent");

    // no tools are registered yet - later commits add read, write, edit,
    // bash, grep, and glob here one at a time
    let dispatcher = Arc::new(Dispatcher::new());

    serve(listener, dispatcher, wait_for_sigterm()).await;

    tracing::info!("munibot_toolagent shut down cleanly");
}
