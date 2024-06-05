pub mod admin;
pub mod commands;
pub mod handler;
pub mod utils;

use std::env;

use dotenvy::dotenv;
use log::{error, info};
use poise::{
    samples::register_globally,
    serenity_prelude::{self as serenity, Settings},
    Prefix, PrefixFrameworkOptions,
};
use surrealdb::{engine::remote::ws, opt::auth::Database, Surreal};

use self::{admin::AdminCommandProvider, commands::DiscordCommandProvider};
use crate::{
    config::{Config, DiscordConfig},
    handlers::DiscordMessageHandlerCollection,
    MuniBotError,
};

pub struct DiscordState {
    pub config: DiscordConfig,
    handlers: DiscordMessageHandlerCollection,
    pub db: Surreal<ws::Client>,
}
impl DiscordState {
    pub async fn new(
        handlers: DiscordMessageHandlerCollection,
        config: &Config,
    ) -> Result<Self, MuniBotError> {
        let database_url = config.db.url.clone();
        let db = Surreal::new::<ws::Ws>(&database_url).await?;
        db.signin(Database {
            namespace: "muni_bot",
            database: "muni_bot",
            username: &config.db.user,
            password: &env::var("DATABASE_PASS").expect("expected DATABASE_PASS to be set"),
        })
        .await?;

        Ok(Self {
            handlers,
            db,
            config: config.discord.clone(),
        })
    }
}

pub type DiscordCommand = poise::Command<DiscordState, MuniBotError>;
pub type DiscordContext<'a> = poise::Context<'a, DiscordState, MuniBotError>;
pub type DiscordFrameworkContext<'a> = poise::FrameworkContext<'a, DiscordState, MuniBotError>;

pub async fn start_discord_integration(
    handlers: DiscordMessageHandlerCollection,
    command_providers: Vec<Box<dyn DiscordCommandProvider>>,
    config: Config,
) {
    dotenv().ok();

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
            Box::pin(on_ready(ctx, ready, framework, handlers, config))
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
) -> Result<DiscordState, MuniBotError> {
    register_globally(ctx, &framework.options().commands)
        .await
        .expect("failed to register commands in guild");

    ctx.set_activity(Some(serenity::ActivityData::watching("you sleep uwu")));

    info!("discord: logged in as {}", ready.user.name);

    DiscordState::new(handlers, &config).await
}

async fn event_handler(
    context: &serenity::Context,
    event: &serenity::FullEvent,
    framework_context: DiscordFrameworkContext<'_>,
    data: &DiscordState,
) -> Result<(), MuniBotError> {
    for handler in data.handlers.iter() {
        let mut locked_handler = handler.lock().await;
        let handled_future = locked_handler.handle_discord_event(context, framework_context, event);
        if let Err(e) = handled_future.await {
            error!(
                "discord integration ran into an error executing handlers: {}",
                e
            );
        }
    }
    Ok(())
}
