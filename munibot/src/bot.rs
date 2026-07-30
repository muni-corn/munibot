// bot.rs: starts and supervises munibot's discord and twitch integrations.
//
// only compiled with the `server` feature, since it depends on the native
// discord/twitch crates, diesel, and tokio -- none of which can target wasm.

use std::sync::Arc;

use munibot_ai::Ai;
use munibot_core::{config::Config, db::establish_pool};
use munibot_discord::{
    DiscordMessageHandlerCollection,
    commands::{DiscordCommandProviderCollection, fox::FoxCommandProvider},
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

/// Starts the discord integration as a background task, returning its join
/// handle. `ai` is `None` when `ai.enabled` is false or unset.
pub fn start_discord(config: Config, ai: Option<Arc<Ai>>) -> tokio::task::JoinHandle<()> {
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
        Box::new(FoxCommandProvider),
    ];

    // attach a root span so all events from within the discord integration
    // carry the "discord" context in the subscriber output
    let span = info_span!("discord");
    tokio::spawn(
        start_discord_integration(discord_handlers, discord_command_providers, config, ai)
            .instrument(span),
    )
}

/// Starts the twitch integration if a `TWITCH_TOKEN` is available, returning
/// its join handle. Logs an auth URL and returns `None` if twitch isn't
/// configured yet.
pub async fn start_twitch(config: &Config) -> Option<tokio::task::JoinHandle<()>> {
    match std::env::var("TWITCH_TOKEN") {
        Ok(twitch_token) => {
            // establish pool for the twitch bot
            let pool = establish_pool()
                .await
                .expect("couldn't establish database connection pool for twitch");

            match TwitchBot::new(pool, config)
                .await
                .launch(twitch_token, config)
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
    }
}

/// Starts the discord and twitch integrations, then spawns a supervisor task
/// that logs if either one unexpectedly stops.
///
/// This must be `.await`ed directly on the caller's task rather than wrapped
/// in its own `tokio::spawn`: setting up twitch briefly holds a `TwitchBot`
/// across an `.await`, and its handler collection isn't `Sync`, so the setup
/// future itself isn't `Send`. The supervisor task spawned internally only
/// captures the resulting `JoinHandle`s, which are always `Send`, so it's
/// safe to run in the background once setup completes.
pub async fn start(config: Config, ai: Option<Arc<Ai>>) {
    let discord_handle = start_discord(config.clone(), ai);
    let twitch_handle = start_twitch(&config).await;

    tokio::spawn(async move {
        // wait for the discord bot to stop, if ever
        match discord_handle.await {
            Ok(_) => warn!("discord bot stopped o.o  this is probably not supposed to happen..."),
            Err(e) => error!(error = %e, "discord bot died"),
        }

        if let Some(twitch_handle) = twitch_handle {
            match twitch_handle.await {
                Ok(_) => {
                    warn!("twitch bot stopped o.o  this is probably not supposed to happen...")
                }
                Err(e) => error!(error = %e, "twitch bot died"),
            }
        }

        warn!("all bot integrations have unexpectedly stopped o.o");
    });
}
