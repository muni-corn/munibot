#[cfg(feature = "server")]
use std::sync::Arc;

#[cfg(feature = "server")]
use clap::Parser;
#[cfg(feature = "server")]
use munibot_ai::{Ai, memory::DieselSessionStore, persona::AiConfig, tools::ToolRegistry};
#[cfg(feature = "server")]
use munibot_core::{
    config::Config,
    db::{establish_pool, run_pending_migrations},
};
#[cfg(feature = "server")]
use tracing::info;
#[cfg(feature = "server")]
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[cfg(feature = "server")]
#[derive(Parser, Debug)]
struct Args {
    /// Path to a config file. Overridden by `MUNIBOT_CONFIG_FILE` if set,
    /// since `dx serve` doesn't forward CLI args to the server binary.
    #[clap(short, long, default_value = "/etc/muni_bot/config.toml")]
    config_file: String,
}

// web entry point — dioxus handles hydration automatically
#[cfg(not(feature = "server"))]
fn main() {
    munibot_gui::launch_web();
}

// server entry point: runs the discord/twitch bots alongside the gui server
#[cfg(feature = "server")]
#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    // initialize the tracing subscriber with an env filter, bridging any
    // log-crate records from transitive dependencies into the tracing pipeline
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();

    let args = Args::parse();
    let config_file = std::env::var("MUNIBOT_CONFIG_FILE").unwrap_or(args.config_file);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_file,
        "starting munibot"
    );

    let config = Config::read_or_write_default_from(&config_file)?;

    // first things first, perform database migrations
    run_pending_migrations();

    // built unconditionally, and independently of the bots below: the gui
    // server needs it too, and `MUNIBOT_DISABLE_BOTS` is the documented
    // local gui development workflow, so gating this on the same guard would
    // leave the chat page with no service behind it
    let ai_config = AiConfig::load_from_file(&config_file)?;
    let ai = if ai_config.enabled {
        let tools = Arc::new(ToolRegistry::from_env());
        // diesel-backed, not in-memory: conversations started through the
        // gui have to survive a server restart, which is the entire point
        // of this milestone
        let pool = establish_pool()
            .await
            .expect("couldn't establish database connection pool for ai");
        let sessions: Arc<dyn munibot_ai::memory::SessionStore> =
            Arc::new(DieselSessionStore::new(pool));
        Some(Arc::new(Ai::new(&ai_config, tools, sessions)?))
    } else {
        info!("ai.enabled is false; skipping ai setup");
        None
    };

    // start the bots alongside the gui server, unless explicitly disabled for
    // local gui development (so `dx serve` reloads don't reconnect discord)
    if std::env::var("MUNIBOT_DISABLE_BOTS").is_err() {
        munibot::bot::start(config.clone(), ai.clone()).await;
    } else {
        info!("MUNIBOT_DISABLE_BOTS is set; skipping discord and twitch startup");
    }

    munibot_gui::server::run(config.discord, ai).await
}
