use std::sync::Arc;

use clap::Parser;
use munibot_core::{
    config::Config,
    db::{establish_pool, run_pending_migrations},
};
use munibot_discord::{
    DiscordMessageHandlerCollection,
    commands::DiscordCommandProviderCollection,
    error::MunibotDiscordError,
    handlers::{
        bot_affection::BotAffectionProvider, dice::DiceHandler, economy::EconomyProvider,
        greeting::GreetingHandler as DiscordGreetingHandler,
        magical::MagicalHandler as DiscordMagicalHandler,
        temperature::TemperatureConversionProvider, ventriloquize::VentriloquizeProvider,
    },
    simple::SimpleCommandProvider,
    start_discord_integration,
    vc_greeter::VoiceChannelGreeter,
};
use munibot_twitch::{TwitchBot, get_basic_auth_url};
use tokio::sync::Mutex;
use tracing::{Instrument, error, info, info_span, warn};
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
    run_pending_migrations().await;

    let discord_handle = start_discord(config.clone());

    // ensure credentials exist
    let twitch_handle = match std::env::var("TWITCH_TOKEN") {
        Ok(twitch_token) => {
            // establish pool for the twitch bot
            let pool = establish_pool()
                .await
                .expect("couldn't establish database connection pool for twitch");

            // start twitch
            match TwitchBot::new(pool, &config)
                .await
                .launch(twitch_token, &config)
                .await
            {
                // wait for the twitch bot to stop, if ever
                Ok(twitch_handle) => Some(twitch_handle),
                Err(e) => {
                    error!(error = %e, "twitch bot failed to start");
                    None
                }
            }
        }
        Err(e) => {
            if let Ok(auth_page_url) = get_basic_auth_url() {
                error!(error = %e, "no TWITCH_TOKEN found");
                info!(url = %auth_page_url, "visit this url to get a token");
            } else {
                error!(
                    "no TWITCH_TOKEN found and no TWITCH_CLIENT_ID set. the TWITCH_CLIENT_ID \
                     environment variable is required to generate an auth url link."
                );
            }
            warn!(
                "since twitch integration is misconfigured, i won't be running my twitch \
                 integration at this time. >.>"
            );
            None
        }
    };

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

fn start_discord(config: Config) -> tokio::task::JoinHandle<()> {
    // start discord
    let discord_handlers: DiscordMessageHandlerCollection = vec![
        Arc::new(Mutex::new(DiscordGreetingHandler)),
        Arc::new(Mutex::new(EconomyProvider)),
        Arc::new(Mutex::new(VoiceChannelGreeter)),
    ];
    let discord_command_providers: DiscordCommandProviderCollection = vec![
        Box::new(DiceHandler),
        Box::new(BotAffectionProvider),
        Box::new(DiscordMagicalHandler),
        Box::new(VentriloquizeProvider),
        Box::new(EconomyProvider),
        Box::new(TemperatureConversionProvider),
        Box::new(SimpleCommandProvider),
    ];

    // attach a root span so all events from within the discord integration
    // carry the "discord" context in the subscriber output
    let span = info_span!("discord");
    tokio::spawn(
        start_discord_integration(discord_handlers, discord_command_providers, config)
            .instrument(span),
    )
}
