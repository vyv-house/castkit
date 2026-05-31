use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use tower_http::services::ServeDir;
use tracing::info;

use crate::room::Rooms;

mod api;
mod ingest;
mod room;
mod watch;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub secret: String,
    pub web_dir: String,
}

pub struct AppState {
    pub rooms: Rooms,
    pub secret: String,
}

pub async fn run_server(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        rooms: Rooms::new(),
        secret: config.secret,
    };

    let app = Router::new()
        .route("/ingest/{room_id}", get(ingest::handle_ingest))
        .route("/watch/{room_id}", get(watch::handle_watch))
        .route("/api/rooms", get(api::list_rooms))
        .route("/api/rooms/{room_id}", get(api::room_detail))
        .fallback_service(ServeDir::new(&config.web_dir))
        .with_state(Arc::new(state));

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(addr = %config.bind_addr, web_dir = %config.web_dir, "gateway listening");
    axum::serve(listener, app).await?;

    Ok(())
}
