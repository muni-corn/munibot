use std::sync::Arc;

use clap::Parser;
use env_logger::Env;
use log::{error, info, warn};
use munibot::{
    MuniBotError,
    config::Config,
    discord::{
        simple::SimpleCommandProvider, start_discord_integration, vc_greeter::VoiceChannelGreeter,
    },
    handlers::{
        DiscordCommandProviderCollection, DiscordMessageHandlerCollection,
        bot_affection::BotAffectionProvider, dice::DiceHandler, economy::EconomyProvider,
        greeting::GreetingHandler, magical::MagicalHandler,
        temperature::TemperatureConversionProvider, ventriloquize::VentriloquizeProvider,
    },
    twitch::{bot::TwitchBot, get_basic_auth_url},
};
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
struct Args {
    /// Path to a config file.
    #[clap(short, long, default_value = "/etc/muni_bot/config.toml")]
    config_file: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<(), Box<MuniBotError>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    let config = Config::read_or_write_default_from(&args.config_file)?;

    let discord_handle = start_discord(config.clone());

    // ensure credentials exist
    let twitch_handle = match std::env::var("TWITCH_TOKEN") {
        Ok(twitch_token) => {
            // start twitch
            match TwitchBot::new(config.clone())
                .await
                .launch(twitch_token, &config)
                .await
            {
                // wait for the twitch bot to stop, if ever
                Ok(twitch_handle) => Some(twitch_handle),
                Err(e) => {
                    error!("twitch bot failed to start :< {e}");
                    None
                }
            }
        }
        Err(e) => {
            if let Ok(auth_page_url) = get_basic_auth_url() {
                error!("no TWITCH_TOKEN found ({e})");
                info!("visit {auth_page_url} to get a token");
            } else {
                error!(
                    "no TWITCH_TOKEN found and no TWITCH_CLIENT_ID set. the TWITCH_CLIENT_ID environment variable is required to generate an auth url link."
                );
            }
            warn!(
                "since twitch integration is misconfigured, i won't be running my twitch integration at this time. >.>"
            );
            None
        }
    };

    // wait for the discord bot to stop, if ever
    match discord_handle.await {
        Ok(_) => warn!("discord bot stopped o.o  this is probably not supposed to happen..."),
        Err(e) => error!("discord bot died with error: {e}"),
    }

    if let Some(twitch_handle) = twitch_handle {
        match twitch_handle.await {
            Ok(_) => warn!("twitch bot stopped o.o  this is probably not supposed to happen..."),
            Err(e) => error!("twitch bot died with error: {e}"),
        }
    }

    warn!(
        "all bot integrations have unexpectedly stopped. i can't do anything else right now. goodbye! ^-^"
    );
    Ok(())
}

fn start_discord(config: Config) -> tokio::task::JoinHandle<()> {
    // start discord
    let discord_handlers: DiscordMessageHandlerCollection = vec![
        Arc::new(Mutex::new(GreetingHandler)),
        Arc::new(Mutex::new(EconomyProvider)),
        Arc::new(Mutex::new(VoiceChannelGreeter)),
    ];
    let discord_command_providers: DiscordCommandProviderCollection = vec![
        Box::new(DiceHandler),
        Box::new(BotAffectionProvider),
        Box::new(MagicalHandler),
        Box::new(VentriloquizeProvider),
        Box::new(EconomyProvider),
        Box::new(TemperatureConversionProvider),
        Box::new(SimpleCommandProvider),
    ];

    tokio::spawn(start_discord_integration(
        discord_handlers,
        discord_command_providers,
        config,
    ))
}
