// server.rs: the axum server that renders the gui, serves munibot_api's
// server functions, and backs login sessions with redis.
//
// only compiled with the `server` feature -- native only, since it depends
// on axum, redis, and munibot_core's diesel pool, none of which can target
// wasm32.

use axum::Extension;
use axum_session::{SessionConfig, SessionLayer, SessionStore};
use axum_session_auth::{AuthConfig, AuthSessionLayer};
use axum_session_redispool::SessionRedisPool;
use dioxus::prelude::*;
use munibot_api::auth::server::User;
use munibot_core::db::establish_pool;
use redis_pool::RedisPool;
use tracing::info;

use crate::app::App;

/// Builds and serves the gui's axum app: the dioxus fullstack app, the
/// discord oauth routes, and the redis-backed session/auth layers.
///
/// Establishes its own db pool: sessions load the current user through it,
/// and it's also injected as an `Extension` for server functions that need
/// direct db access (e.g. fetching a linked account's oauth token).
pub async fn run() -> anyhow::Result<()> {
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
        .serve_dioxus_application(ServeConfig::new(), App)
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
