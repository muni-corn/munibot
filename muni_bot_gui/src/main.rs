use axum::{
    body::Body,
    extract::{FromRef, Request, State},
    response::{IntoResponse, Response},
};
use clap::Parser;
use leptos::{provide_context, LeptosOptions};
use leptos_axum::handle_server_fns_with_context;
use leptos_router::RouteListing;
use muni_bot_gui::{app::App, auth::ssr::AuthSession};
use surrealdb::{engine::remote::ws, Surreal};

#[cfg(feature = "ssr")]
#[derive(FromRef, Debug, Clone)]
struct AppState {
    leptos_options: LeptosOptions,
    pool: Surreal<ws::Client>,
    routes: Vec<RouteListing>,
}

async fn server_fn_handler(
    State(app_state): State<AppState>,
    auth_session: AuthSession,
    request: Request<Body>,
) -> impl IntoResponse {
    handle_server_fns_with_context(
        move || {
            provide_context(auth_session.clone());
            provide_context(app_state.pool.clone());
        },
        request,
    )
    .await
}

async fn leptos_routes_handler(
    auth_session: AuthSession,
    State(app_state): State<AppState>,
    req: Request<Body>,
) -> Response {
    let handler = leptos_axum::render_route_with_context(
        app_state.leptos_options.clone(),
        app_state.routes.clone(),
        move || {
            provide_context(auth_session.clone());
            provide_context(app_state.pool.clone());
        },
        App,
    );
    handler(req).await.into_response()
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to a config file.
    #[clap(short, long, default_value = "/etc/muni_bot/config.toml")]
    config_file: String,
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    use axum::{routing::get, Router};
    use axum_session::{SessionConfig, SessionLayer};
    use axum_session_auth::{AuthConfig, AuthSessionLayer};
    use axum_session_surreal::{SessionSurrealPool, SessionSurrealSessionStore};
    use leptos::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use muni_bot_gui::{
        app::*, auth::ssr::User, fileserv::file_and_error_handler, utils::ssr::connect_to_db,
    };
    use tokio::net::TcpListener;

    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let args = Args::parse();
    let muni_bot_config =
        muni_bot_core::config::Config::read_or_write_default_from(&args.config_file)?;

    // Setting get_configuration(None) means we'll be using cargo-leptos's env
    // values For deployment these variables are:
    // <https://github.com/leptos-rs/start-axum#executing-a-server-on-a-remote-machine-without-the-toolchain>
    // Alternately a file can be specified such as Some("Cargo.toml")
    // The file would need to be included with the executable when moved to
    // deployment
    let conf = get_configuration(None).await.unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    let pool = connect_to_db(&muni_bot_config.db).await.unwrap();

    let app_state = AppState {
        leptos_options,
        pool: pool.clone(),
        routes: routes.clone(),
    };

    let session_store = SessionSurrealSessionStore::new(
        Some(SessionSurrealPool::new(pool.clone())),
        SessionConfig::default().with_table_name("axum_session"),
    )
    .await?;

    let auth_config = AuthConfig::default();

    // build our application with a route
    let app = Router::new()
        .route(
            "/api/*fn_name",
            get(server_fn_handler).post(server_fn_handler),
        )
        .leptos_routes_with_handler(routes, get(leptos_routes_handler))
        .fallback(file_and_error_handler)
        .layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<ws::Client>, Surreal<ws::Client>>::new(
                Some(pool.clone()),
            )
            .with_config(auth_config),
        )
        .layer(SessionLayer::new(session_store))
        .with_state(app_state);

    let listener = TcpListener::bind(&addr).await.unwrap();
    logging::log!("listening on http://{}", &addr);
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for a purely client-side app
    // see lib.rs for hydration function instead
}
