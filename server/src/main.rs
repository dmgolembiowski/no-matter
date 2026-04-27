//! no-matter server: HTTP API + WebSocket gateway.
//!
//! Defaults are dev-friendly: SQLite at `./no-matter.db` in the CWD,
//! listening on `0.0.0.0:8080`, permissive CORS so the desktop client
//! (Tauri webview) can talk to it from any origin.
//!
//! Override with env vars:
//!   - `DATABASE_URL` — any URL sea-orm understands. Default
//!     `sqlite://./no-matter.db?mode=rwc` (auto-creates the file).
//!   - `LISTEN_ADDR` — host:port to bind. Default `0.0.0.0:8080`.

mod auth;
mod channels;
mod entities;
mod error;
mod files;
mod messages;
mod state;
mod ws;

use std::net::SocketAddr;

use axum::{
    routing::{get, post},
    Router,
};
use migration::MigratorTrait;
use sea_orm::{ConnectOptions, Database};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,no_matter_server=debug,sea_orm=warn".into()),
        )
        .init();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://./no-matter.db?mode=rwc".to_string());

    let mut opts = ConnectOptions::new(&db_url);
    opts.sqlx_logging(false);
    let db = Database::connect(opts).await?;
    tracing::info!("connected to {db_url}");

    migration::Migrator::up(&db, None).await?;
    tracing::info!("migrations applied");

    let state = AppState::new(db);

    // Permissive CORS — single-tenant desktop app, the user controls
    // both ends. Tightening this is a hardening pass, not an MVP need.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api = Router::new()
        .route("/api/signup", post(auth::signup))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/initial_state", get(channels::initial_state))
        .route("/api/check_channel_name", post(channels::check_name))
        .route("/api/create_channel", post(channels::create_channel))
        .route("/api/create_group", post(channels::create_group))
        .route("/api/add_member", post(channels::add_member))
        .route("/api/open_dm", post(channels::open_dm))
        .route("/api/load_messages", post(messages::load_messages))
        .route("/api/post_message", post(messages::post_message))
        .route("/api/edit_message", post(messages::edit_message))
        .route("/api/delete_message", post(messages::delete_message))
        .route("/api/mark_channel_read", post(messages::mark_channel_read))
        .route("/files", post(files::upload))
        .route("/files/{id}", get(files::get_file))
        .route("/api/get_file_meta", post(files::get_file_meta))
        .route("/ws", get(ws::gateway::upgrade));

    let app = api
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    let addr: SocketAddr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    tracing::info!("listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
