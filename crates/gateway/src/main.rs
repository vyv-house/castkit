use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use clap::Parser;
use tower_http::services::ServeDir;
use tracing::info;

use crate::room::Rooms;

mod api;
mod ingest;
mod room;
mod watch;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "8080")]
    port: u16,
    #[arg(long, default_value = "changeme")]
    secret: String,
    #[arg(long, default_value = "../../../web")]
    web_dir: String,
}

pub struct AppState {
    pub rooms: Rooms,
    pub secret: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let state = AppState {
        rooms: Rooms::new(),
        secret: args.secret.clone(),
    };

    let app = Router::new()
        .route("/ingest/{room_id}", get(ingest::handle_ingest))
        .route("/watch/{room_id}", get(watch::handle_watch))
        .route("/api/rooms", get(api::list_rooms))
        .route("/api/rooms/{room_id}", get(api::room_detail))
        .fallback_service(ServeDir::new(&args.web_dir))
        .with_state(Arc::new(state));

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, web_dir = %args.web_dir, "gateway listening");
    axum::serve(listener, app).await?;

    Ok(())
}
