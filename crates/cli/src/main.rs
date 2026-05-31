use std::net::SocketAddr;

use clap::Parser;
use gateway::{run_server, ServerConfig};
use producer::{run_producer, ProducerConfig};

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn BackingScaleFactor(_display: u32) -> f32 {
    1.0
}

#[derive(clap::Parser)]
#[command(name = "castkit", about = "Low-latency screen sharing")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Share your screen to a castkit server
    Share(ShareArgs),
    /// Start the relay gateway server
    Serve(ServeArgs),
}

#[derive(clap::Args)]
struct ShareArgs {
    #[arg(long)]
    room: String,
    #[arg(long, default_value = "ws://127.0.0.1:8080")]
    server: String,
    #[arg(long, env = "CASTKIT_TOKEN")]
    token: Option<String>,
    #[arg(long, default_value_t = 30)]
    fps: u32,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
}

#[derive(clap::Args)]
struct ServeArgs {
    #[arg(long, default_value = "8080")]
    port: u16,
    #[arg(long, env = "CASTKIT_SECRET")]
    secret: Option<String>,
    #[arg(long, default_value = "web")]
    web_dir: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    match Cli::parse().command {
        Command::Share(args) => {
            run_producer(ProducerConfig::new(
                args.room,
                args.server,
                args.token,
                args.fps,
                args.width,
                args.height,
            ))
            .await
        }
        Command::Serve(args) => {
            run_server(ServerConfig {
                bind_addr: SocketAddr::from(([0, 0, 0, 0], args.port)),
                secret: args.secret.unwrap_or_default(),
                web_dir: args.web_dir,
            })
            .await
        }
    }
}
