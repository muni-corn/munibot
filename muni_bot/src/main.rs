use std::sync::Arc;

use clap::Parser;
use env_logger::Env;
use log::{error, info, warn};
use muni_bot::{
    config::Config,
    discord::start_discord_integration,
    handlers::{
        bot_affection::BotAffectionProvider, dice::DiceHandler, economy::EconomyProvider,
        greeting::GreetingHandler, logging::LoggingHandler, magical::MagicalHandler,
        temperature::TemperatureConversionProvider, ventriloquize::VentriloquizeProvider,
        DiscordCommandProviderCollection, DiscordMessageHandlerCollection,
    },
    twitch::{bot::TwitchBot, get_basic_auth_url},
    MuniBotError,
};
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
struct Args {
    /// Path to a config file.
    #[clap(short, long, default_value = "/etc/muni_bot/config.toml")]
    config_file: String,
}

#[tokio::main]
async fn main() -> Result<(), MuniBotError> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    let config = Config::read_or_write_default_from(&args.config_file)?;

    // ensure credentials exist
    match std::env::var("TWITCH_TOKEN") {
        Ok(twitch_token) => {
            let discord_handle = start_discord(config.clone());

            // start twitch
            match TwitchBot::new(config.clone())
                .await
                .start(twitch_token, &config)
                .await
            {
                // wait for the twitch bot to stop, if ever
                Ok(twitch_handle) => match twitch_handle.await {
                    Ok(_) => warn!("twitch bot stopped o.o"),
                    Err(e) => error!("twitch bot died with error: {e}"),
                },
                Err(e) => error!("twitch bot failed to start :< {e}"),
            }

            // wait for the discord bot to stop, if ever
            match discord_handle.await {
                Ok(_) => warn!("discord bot stopped o.o"),
                Err(e) => error!("discord bot died with error: {e}"),
            }

            info!("all bot integrations have stopped. goodbye ^-^");
            Ok(())
        }
        Err(e) => {
            let auth_page_url = get_basic_auth_url();
            error!("no TWITCH_TOKEN found ({e})");
            info!("visit {auth_page_url} to get a token");
            Err(MuniBotError::MissingToken)
        }
    }
}

fn start_discord(config: Config) -> tokio::task::JoinHandle<()> {
    // start discord
    let discord_handlers: DiscordMessageHandlerCollection = vec![
        Arc::new(Mutex::new(GreetingHandler)),
        Arc::new(Mutex::new(EconomyProvider)),
        Arc::new(Mutex::new(LoggingHandler)),
    ];
    let discord_command_providers: DiscordCommandProviderCollection = vec![
        Box::new(DiceHandler),
        Box::new(BotAffectionProvider),
        Box::new(MagicalHandler),
        Box::new(VentriloquizeProvider),
        Box::new(EconomyProvider),
        Box::new(TemperatureConversionProvider),
    ];

    tokio::spawn(start_discord_integration(
        discord_handlers,
        discord_command_providers,
        config,
    ))
}
