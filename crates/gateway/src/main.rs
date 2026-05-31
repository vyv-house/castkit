use std::net::SocketAddr;

use clap::Parser;

use gateway::{run_server, ServerConfig};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "8080")]
    port: u16,
    #[arg(long, default_value = "changeme")]
    secret: String,
    #[arg(long, default_value = "../../../web")]
    web_dir: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    run_server(ServerConfig {
        bind_addr: SocketAddr::from(([0, 0, 0, 0], args.port)),
        secret: args.secret,
        web_dir: args.web_dir,
    })
    .await
}
