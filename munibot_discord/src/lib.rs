#![feature(never_type)]

use dotenvy::dotenv;
use log::{error, info};
use munibot_core::{
    config::Config,
    db::{DbPool, establish_pool},
};
use poise::{
    Prefix, PrefixFrameworkOptions,
    samples::register_globally,
    serenity_prelude::{self as serenity, Settings},
};

pub mod admin;
pub mod autodelete;
pub mod commands;
pub mod error;
pub mod handler;
pub mod handlers;
pub mod simple;
pub mod state;
pub mod utils;
pub mod vc_greeter;

pub use error::MuniBotError as DiscordError;
pub use state::{DiscordMessageHandlerCollection, DiscordState};

use crate::{
    admin::AdminCommandProvider,
    autodelete::AutoDeleteHandler,
    commands::{DiscordCommandProvider, DiscordCommandProviderCollection},
    error::MuniBotError,
};

/// Poise command type for the discord integration.
pub type DiscordCommand = poise::Command<DiscordState, MuniBotError>;
/// Poise context type for the discord integration.
pub type DiscordContext<'a> = poise::Context<'a, DiscordState, MuniBotError>;
/// Poise framework context type for the discord integration.
pub type DiscordFrameworkContext<'a> = poise::FrameworkContext<'a, DiscordState, MuniBotError>;

/// Starts the Discord integration. Runs until the Discord client stops.
pub async fn start_discord_integration(
    handlers: DiscordMessageHandlerCollection,
    command_providers: DiscordCommandProviderCollection,
    config: Config,
) {
    dotenv().ok();

    // establish the MySQL connection pool
    let pool = establish_pool()
        .await
        .expect("couldn't establish database connection pool");

    let mut commands: Vec<DiscordCommand> = command_providers
        .iter()
        .flat_map(|provider| provider.commands())
        .collect();

    // always add admin commands
    commands.append(&mut AdminCommandProvider.commands());

    let options = poise::FrameworkOptions::<DiscordState, MuniBotError> {
        event_handler: |ctx, event, framework, data| {
            Box::pin(event_handler(ctx, event, framework, data))
        },
        commands,
        prefix_options: PrefixFrameworkOptions {
            prefix: Some("~".to_string()),
            additional_prefixes: vec![Prefix::Literal("!")],
            ..Default::default()
        },
        ..Default::default()
    };

    let token = std::env::var("DISCORD_TOKEN")
        .expect("no token provided for discord! i can't run without it :(");
    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    let framework = poise::Framework::<DiscordState, MuniBotError>::builder()
        .setup(move |ctx, ready, framework| {
            Box::pin(on_ready(ctx, ready, framework, handlers, config, pool))
        })
        .options(options)
        .build();

    // create cache settings
    let mut cache_settings = Settings::default();
    cache_settings.max_messages = 10000;

    // `await`ing builds the client
    let mut client = serenity::ClientBuilder::new(token, intents)
        .cache_settings(cache_settings)
        .framework(framework)
        .await
        .unwrap();

    client.start().await.unwrap();
}

async fn on_ready(
    ctx: &serenity::Context,
    ready: &serenity::Ready,
    framework: &poise::Framework<DiscordState, MuniBotError>,
    handlers: DiscordMessageHandlerCollection,
    config: Config,
    pool: DbPool,
) -> serenity::Result<DiscordState, MuniBotError> {
    register_globally(ctx, &framework.options().commands)
        .await
        .expect("failed to register commands globally");

    ctx.set_activity(Some(serenity::ActivityData::watching("you sleep uwu")));

    info!("discord: logged in as {}", ready.user.name);

    let new_state =
        DiscordState::new(handlers, &config, pool, ctx.http.clone(), ctx.cache.clone()).await?;

    // start the autodeletion handler
    AutoDeleteHandler::start(new_state.autodeletion().clone());

    Ok(new_state)
}

async fn event_handler(
    context: &serenity::Context,
    event: &serenity::FullEvent,
    framework_context: DiscordFrameworkContext<'_>,
    data: &DiscordState,
) -> Result<(), MuniBotError> {
    for handler in data.handlers().iter() {
        let mut locked_handler = handler.lock().await;
        let handled_future = locked_handler.handle_discord_event(context, framework_context, event);
        if let Err(e) = handled_future.await {
            error!("discord: error in {} handler: {}", locked_handler.name(), e);
        }
    }
    Ok(())
}
