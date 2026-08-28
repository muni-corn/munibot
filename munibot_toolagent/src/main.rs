//! The in-container rpc server executing filesystem and shell tools for the
//! ai sandbox (`munibot_ai::sandbox`).
//!
//! Baked into the sandbox container image and started once per sandbox by
//! the host, over a unix socket mounted from a host tmpfs directory. Takes
//! no munibot dependency at all -- see this crate's `Cargo.toml` for why.

use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
struct Args {
    /// Path to the unix socket to listen on, mounted into the container by
    /// the host.
    #[clap(long)]
    socket: String,
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();

    let args = Args::parse();

    tracing::info!(socket = %args.socket, "starting munibot_toolagent");
}
