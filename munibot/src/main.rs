#[cfg(feature = "server")]
use axum::Extension;
#[cfg(feature = "server")]
use axum_session::{SessionConfig, SessionLayer, SessionStore};
#[cfg(feature = "server")]
use axum_session_auth::{AuthConfig, AuthSessionLayer};
#[cfg(feature = "server")]
use axum_session_redispool::SessionRedisPool;
#[cfg(feature = "server")]
use clap::Parser;
#[cfg(feature = "server")]
use dioxus::prelude::*;
#[cfg(feature = "server")]
use munibot_api::auth::server::User;
#[cfg(feature = "server")]
use munibot_core::{
    config::Config,
    db::{establish_pool, run_pending_migrations},
};
#[cfg(feature = "server")]
use redis_pool::RedisPool;
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
    dioxus::launch(munibot::app::App);
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

    // start the bots alongside the gui server, unless explicitly disabled for
    // local gui development (so `dx serve` reloads don't reconnect discord)
    if std::env::var("MUNIBOT_DISABLE_BOTS").is_err() {
        munibot::bot::start(config.clone()).await;
    } else {
        info!("MUNIBOT_DISABLE_BOTS is set; skipping discord and twitch startup");
    }

    // the gui's own db pool: sessions load the current user through this,
    // and it's also injected as an Extension for server functions that need
    // direct db access (e.g. fetching a linked account's oauth token)
    let pool = establish_pool()
        .await
        .expect("couldn't establish database connection pool for the gui");

    // login sessions live in redis, keyed by an opaque session id cookie
    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis_client = redis::Client::open(redis_url).expect("couldn't create redis client");
    let session_pool = SessionRedisPool::from(RedisPool::from(redis_client));
    let session_store = SessionStore::<SessionRedisPool>::new(
        Some(session_pool),
        SessionConfig::default().with_table_name("munibot_sessions"),
    )
    .await?;

    let address = dioxus::cli_config::fullstack_address_or_localhost();
    let app = axum::Router::new()
        .serve_dioxus_application(ServeConfig::new(), munibot::app::App)
        // merged before the layers below so sign-in/logout see the same
        // session + db state as everything else
        .merge(munibot_api::oauth::routes::router())
        .layer(
            AuthSessionLayer::<User, String, SessionRedisPool, _>::new(Some(pool.clone()))
                .with_config(AuthConfig::<String>::default()),
        )
        .layer(SessionLayer::new(session_store))
        .layer(Extension(pool));

    let listener = tokio::net::TcpListener::bind(address).await?;
    info!(address = %address, "listening");
    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}
