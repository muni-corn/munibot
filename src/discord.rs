pub mod admin;
pub mod autodelete;
pub mod commands;
pub mod handler;
pub mod simple;
pub mod state;
pub mod utils;
pub mod vc_greeter;

use std::{env, sync::Arc};

use autodelete::AutoDeleteHandler;
use dotenvy::dotenv;
use log::{error, info};
use poise::{
    Prefix, PrefixFrameworkOptions,
    samples::register_globally,
    serenity_prelude::{self as serenity, Result, Settings},
};
use state::DiscordState;
use surrealdb::{Surreal, engine::remote::ws, opt::auth::Database};

use self::{admin::AdminCommandProvider, commands::DiscordCommandProvider};
use crate::{MuniBotError, config::Config, handlers::DiscordMessageHandlerCollection};

pub type DiscordCommand = poise::Command<DiscordState, MuniBotError>;
pub type DiscordContext<'a> = poise::Context<'a, DiscordState, MuniBotError>;
pub type DiscordFrameworkContext<'a> = poise::FrameworkContext<'a, DiscordState, MuniBotError>;

pub async fn start_discord_integration(
    handlers: DiscordMessageHandlerCollection,
    command_providers: Vec<Box<dyn DiscordCommandProvider>>,
    config: Config,
) {
    dotenv().ok();

    // login to the database first
    let database_url = config.db.url.clone();
    let db = Surreal::new::<ws::Ws>(&database_url)
        .await
        .expect("couldn't connect to database");
    db.signin(Database {
        namespace: "muni_bot",
        database: "muni_bot",
        username: &config.db.user,
        password: &std::env::var("DATABASE_PASS").expect("expected DATABASE_PASS to be set"),
    })
    .await
    .expect("couldn't log in to database");

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

    let token = env::var("DISCORD_TOKEN")
        .expect("no token provided for discord! i can't run without it :(");
    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    let framework = poise::Framework::<DiscordState, MuniBotError>::builder()
        .setup(move |ctx, ready, framework| {
            Box::pin(on_ready(
                ctx,
                ready,
                framework,
                handlers,
                config,
                Arc::new(db),
            ))
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
    db: Arc<Surreal<ws::Client>>,
) -> Result<DiscordState, MuniBotError> {
    register_globally(ctx, &framework.options().commands)
        .await
        .expect("failed to register commands globally");

    ctx.set_activity(Some(serenity::ActivityData::watching("you sleep uwu")));

    info!("discord: logged in as {}", ready.user.name);

    let new_state =
        DiscordState::new(handlers, &config, db, ctx.http.clone(), ctx.cache.clone()).await?;

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
