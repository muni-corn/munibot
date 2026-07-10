use clap::Parser;
use munibot_core::{config::Config, db::run_pending_migrations};
use munibot_discord::error::MunibotDiscordError;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
struct Args {
    /// Path to a config file.
    #[clap(short, long, default_value = "/etc/muni_bot/config.toml")]
    config_file: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<(), Box<MunibotDiscordError>> {
    dotenvy::dotenv().ok();

    // initialize the tracing subscriber with an env filter, bridging any
    // log-crate records from transitive dependencies into the tracing pipeline
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();

    let args = Args::parse();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %args.config_file,
        "starting munibot"
    );

    let config = Config::read_or_write_default_from(&args.config_file)
        .map_err(|e| Box::new(MunibotDiscordError::Core(*e)))?;

    // first things first, perform database migrations
    run_pending_migrations();

    let discord_handle = munibot::bot::start_discord(config.clone());
    let twitch_handle = munibot::bot::start_twitch(&config).await;

    // wait for the discord bot to stop, if ever
    match discord_handle.await {
        Ok(_) => warn!("discord bot stopped o.o  this is probably not supposed to happen..."),
        Err(e) => error!(error = %e, "discord bot died"),
    }

    if let Some(twitch_handle) = twitch_handle {
        match twitch_handle.await {
            Ok(_) => warn!("twitch bot stopped o.o  this is probably not supposed to happen..."),
            Err(e) => error!(error = %e, "twitch bot died"),
        }
    }

    warn!(
        "all bot integrations have unexpectedly stopped. i can't do anything else right now. \
         goodbye! ^-^"
    );
    Ok(())
}
