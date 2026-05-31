use clap::Parser;
use producer::{run_producer, ProducerConfig, QualityProfile, ResolutionPreset};

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn BackingScaleFactor(_display: u32) -> f32 {
    1.0
}

#[derive(Parser)]
struct Args {
    #[arg(long)]
    room_id: String,
    #[arg(long, default_value = "ws://127.0.0.1:8080")]
    gateway_url: String,
    #[arg(long, default_value = "changeme")]
    secret: String,
    #[arg(long, default_value_t = 30)]
    fps: u32,
    #[arg(long, value_enum, default_value = "balanced")]
    profile: QualityProfile,
    #[arg(long, value_enum, default_value = "source")]
    resolution: ResolutionPreset,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    quality: Option<f32>,
    #[arg(long)]
    keyframe_interval: Option<u32>,
    #[arg(long)]
    capture_queue_depth: Option<usize>,
    #[arg(long)]
    stream_queue_length: Option<i8>,
    #[arg(long, default_value_t = 250)]
    write_timeout_ms: u64,
    #[arg(long)]
    stretch: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    tracing_subscriber::fmt::init();

    run_producer(ProducerConfig {
        room: args.room_id,
        server_url: args.gateway_url,
        token: Some(args.secret),
        fps: args.fps,
        profile: args.profile,
        resolution: args.resolution,
        width: args.width,
        height: args.height,
        quality: args.quality,
        keyframe_interval: args.keyframe_interval,
        capture_queue_depth: args.capture_queue_depth,
        stream_queue_length: args.stream_queue_length,
        write_timeout_ms: args.write_timeout_ms,
        stretch: args.stretch,
    })
    .await
}
