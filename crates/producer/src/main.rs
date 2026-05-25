use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use tokio_tungstenite::tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn BackingScaleFactor(_display: u32) -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum QualityProfile {
    LowLatency,
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ResolutionPreset {
    Source,
    P720,
    P1080,
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

#[derive(Clone, Copy, Debug)]
struct ProducerPolicy {
    fps: u32,
    quality: f32,
    keyframe_interval: u32,
    capture_queue_depth: usize,
    stream_queue_length: i8,
    write_timeout: Duration,
    cpu_used: i32,
    preserve_aspect_ratio: bool,
}

impl ProducerPolicy {
    fn from_args(args: &Args) -> Result<Self, String> {
        if args.fps == 0 {
            return Err("fps must be greater than zero".to_owned());
        }

        let profile = ProfileDefaults::new(args.profile, args.fps);
        let quality = args.quality.unwrap_or(profile.quality);
        if !(0.1..=2.0).contains(&quality) {
            return Err("quality must be between 0.1 and 2.0".to_owned());
        }
        if args.write_timeout_ms == 0 {
            return Err("write-timeout-ms must be greater than zero".to_owned());
        }

        let keyframe_interval = args
            .keyframe_interval
            .unwrap_or(profile.keyframe_interval)
            .max(1);
        let capture_queue_depth = args
            .capture_queue_depth
            .unwrap_or(profile.capture_queue_depth)
            .clamp(1, 8);
        let stream_queue_length = args
            .stream_queue_length
            .unwrap_or(profile.stream_queue_length)
            .clamp(1, 8);

        Ok(Self {
            fps: args.fps,
            quality,
            keyframe_interval,
            capture_queue_depth,
            stream_queue_length,
            write_timeout: Duration::from_millis(args.write_timeout_ms),
            cpu_used: profile.cpu_used,
            preserve_aspect_ratio: !args.stretch,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct ProfileDefaults {
    quality: f32,
    keyframe_interval: u32,
    capture_queue_depth: usize,
    stream_queue_length: i8,
    cpu_used: i32,
}

impl ProfileDefaults {
    fn new(profile: QualityProfile, fps: u32) -> Self {
        match profile {
            QualityProfile::LowLatency => Self {
                quality: 0.75,
                keyframe_interval: fps.max(1),
                capture_queue_depth: 1,
                stream_queue_length: 2,
                cpu_used: 14,
            },
            QualityProfile::Balanced => Self {
                quality: 1.0,
                keyframe_interval: fps.max(1),
                capture_queue_depth: 2,
                stream_queue_length: 3,
                cpu_used: 12,
            },
            QualityProfile::Quality => Self {
                quality: 1.25,
                keyframe_interval: fps.saturating_mul(2).max(1),
                capture_queue_depth: 3,
                stream_queue_length: 4,
                cpu_used: 8,
            },
        }
    }
}

struct FrameData {
    bgra: Vec<u8>,
    stride: usize,
    width: usize,
    height: usize,
}

impl capture::TraitPixelBuffer for FrameData {
    fn data(&self) -> &[u8] {
        &self.bgra
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn stride(&self) -> Vec<usize> {
        vec![self.stride]
    }

    fn pixfmt(&self) -> capture::Pixfmt {
        capture::Pixfmt::BGRA
    }
}

type CaptureResult<T> = Result<T, String>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    tracing_subscriber::fmt::init();

    let policy = ProducerPolicy::from_args(&args).map_err(invalid_input)?;

    let display = capture::Display::primary();
    let (cap_width, cap_height) = target_resolution(&args, &display).map_err(invalid_input)?;

    let encoder = capture::Vp8Encoder::new(capture::Vp8EncoderConfig {
        width: cap_width as u32,
        height: cap_height as u32,
        quality: policy.quality,
        keyframe_interval: Some(policy.keyframe_interval),
        cpu_used: policy.cpu_used,
    })?;

    tracing::info!(
        profile = ?args.profile,
        resolution = ?args.resolution,
        width = cap_width,
        height = cap_height,
        fps = policy.fps,
        quality = policy.quality,
        keyframe_interval = policy.keyframe_interval,
        capture_queue_depth = policy.capture_queue_depth,
        stream_queue_length = policy.stream_queue_length,
        write_timeout_ms = policy.write_timeout.as_millis(),
        cpu_used = policy.cpu_used,
        preserve_aspect_ratio = policy.preserve_aspect_ratio,
        "producer policy"
    );

    let ws_url = format!(
        "{}/ingest/{}?token={}",
        args.gateway_url.trim_end_matches('/'),
        args.room_id,
        args.secret
    );
    let (ws_stream, _) = tokio::task::spawn_blocking(move || connect(ws_url))
        .await?
        .map_err(|error| runtime_error(format!("websocket connect failed: {error}")))?;

    let (frame_tx, frame_rx) = crossbeam_channel::bounded(policy.capture_queue_depth);
    let dropped_capture_frames = Arc::new(AtomicU64::new(0));
    let config = capture::Config {
        cursor: true,
        letterbox: policy.preserve_aspect_ratio,
        throttle: 1.0 / policy.fps as f64,
        queue_length: policy.stream_queue_length,
    };

    let capture_drops = dropped_capture_frames.clone();
    let capturer = capture::Capturer::new(
        display,
        cap_width,
        cap_height,
        capture::PixelFormat::Argb8888,
        config,
        move |mut frame: capture::Frame| {
            frame.surface_to_bgra(cap_height);
            let bgra = frame.to_vec();
            let stride = frame.stride();
            if frame_tx
                .try_send(FrameData {
                    bgra,
                    stride,
                    width: cap_width,
                    height: cap_height,
                })
                .is_err()
            {
                capture_drops.fetch_add(1, Ordering::Relaxed);
            }
        },
    )
    .map_err(|error| runtime_error(format!("failed to start capture: {error:?}")))?;

    let force_kf_flag = Arc::new(AtomicBool::new(false));
    let capture_loop = tokio::task::spawn_blocking(move || {
        run_capture_loop(
            frame_rx,
            encoder,
            ws_stream,
            force_kf_flag,
            cap_width,
            cap_height,
            dropped_capture_frames,
            policy,
        )
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
        }
        result = capture_loop => {
            result?.map_err(runtime_error)?;
        }
    }

    drop(capturer);
    Ok(())
}

fn target_resolution(args: &Args, display: &capture::Display) -> Result<(usize, usize), String> {
    let (preset_width, preset_height) = match args.resolution {
        ResolutionPreset::Source => (display.width(), display.height()),
        ResolutionPreset::P720 => (1280, 720),
        ResolutionPreset::P1080 => (1920, 1080),
    };

    let width = args
        .width
        .map(|width| width as usize)
        .unwrap_or(preset_width);
    let height = args
        .height
        .map(|height| height as usize)
        .unwrap_or(preset_height);
    let width = width.max(1);
    let height = height.max(1);
    if width > u16::MAX as usize || height > u16::MAX as usize {
        return Err("width and height must fit the 16-bit wire protocol".to_owned());
    }

    Ok((width, height))
}

fn invalid_input(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

fn runtime_error(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

fn run_capture_loop(
    frame_rx: crossbeam_channel::Receiver<FrameData>,
    mut encoder: capture::Vp8Encoder,
    mut ws_stream: WebSocket<MaybeTlsStream<TcpStream>>,
    force_kf_flag: Arc<AtomicBool>,
    cap_width: usize,
    cap_height: usize,
    dropped_capture_frames: Arc<AtomicU64>,
    policy: ProducerPolicy,
) -> CaptureResult<()> {
    configure_socket_timeouts(&ws_stream, policy.write_timeout)?;
    let start = Instant::now();
    let mut yuv_buf = Vec::new();
    let mut frame_count: u64 = 0;
    let mut stats = ProducerStats::default();
    let mut last_stats_at = Instant::now();
    loop {
        let mut frame_data = match frame_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(frame) => frame,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };

        let mut stale_frames_dropped = 0;
        while let Ok(next_frame) = frame_rx.try_recv() {
            frame_data = next_frame;
            stale_frames_dropped += 1;
        }
        stats.stale_frames_dropped += stale_frames_dropped;

        poll_force_keyframe(&mut ws_stream, &force_kf_flag)?;

        let yuvfmt = encoder.yuvfmt.clone();
        let convert_started = Instant::now();
        capture::convert_to_yuv(&frame_data, &yuvfmt, &mut yuv_buf)
            .map_err(|error| error.to_string())?;
        stats.convert_ns += convert_started.elapsed().as_nanos() as u64;

        let pts = start.elapsed().as_millis() as i64;
        let force_kf = frame_count % policy.keyframe_interval as u64 == 0
            || force_kf_flag.swap(false, Ordering::Relaxed);
        let encode_started = Instant::now();
        let packets = if force_kf {
            encoder
                .force_keyframe(pts, &yuv_buf)
                .map_err(|error| error.to_string())?
        } else {
            encoder
                .encode(pts, &yuv_buf)
                .map_err(|error| error.to_string())?
        };
        stats.encode_ns += encode_started.elapsed().as_nanos() as u64;

        for packet in &packets {
            let frame_type = if packet.is_keyframe {
                shared::FRAME_KEYFRAME
            } else {
                shared::FRAME_DELTA
            };
            let header = shared::FrameHeader {
                frame_type,
                timestamp_ms: start.elapsed().as_millis() as u32,
                width: cap_width as u16,
                height: cap_height as u16,
            };
            let msg = shared::encode_frame(&header, &packet.data);
            stats.bytes_sent += msg.len() as u64;
            let send_started = Instant::now();
            ws_stream
                .send(Message::Binary(msg))
                .map_err(|error| error.to_string())?;
            stats.send_ns += send_started.elapsed().as_nanos() as u64;
            stats.packets_sent += 1;
            if packet.is_keyframe {
                stats.keyframes_sent += 1;
            }
        }

        frame_count += 1;
        stats.frames_captured += 1;

        if last_stats_at.elapsed() >= Duration::from_secs(1) {
            let elapsed = last_stats_at.elapsed().as_secs_f64();
            let dropped = dropped_capture_frames.swap(0, Ordering::Relaxed);
            stats.log(elapsed, dropped);
            stats = ProducerStats::default();
            last_stats_at = Instant::now();
        }
    }

    Ok(())
}

#[derive(Default)]
struct ProducerStats {
    frames_captured: u64,
    packets_sent: u64,
    keyframes_sent: u64,
    stale_frames_dropped: u64,
    bytes_sent: u64,
    convert_ns: u64,
    encode_ns: u64,
    send_ns: u64,
}

impl ProducerStats {
    fn log(&self, elapsed_secs: f64, dropped_capture_frames: u64) {
        let frames = self.frames_captured.max(1);
        let pipeline_busy_pct = (self.convert_ns + self.encode_ns + self.send_ns) as f64
            / (elapsed_secs * 1_000_000_000.0)
            * 100.0;
        tracing::info!(
            fps = self.frames_captured as f64 / elapsed_secs,
            pipeline_busy_pct,
            packets_sent = self.packets_sent,
            keyframes_sent = self.keyframes_sent,
            stale_frames_dropped = self.stale_frames_dropped,
            dropped_capture_frames,
            kbps = (self.bytes_sent as f64 * 8.0 / 1000.0) / elapsed_secs,
            avg_convert_ms = self.convert_ns as f64 / frames as f64 / 1_000_000.0,
            avg_encode_ms = self.encode_ns as f64 / frames as f64 / 1_000_000.0,
            avg_send_ms = self.send_ns as f64 / self.packets_sent.max(1) as f64 / 1_000_000.0,
            "producer telemetry"
        );
    }
}

fn poll_force_keyframe<S>(
    ws_stream: &mut WebSocket<S>,
    force_kf_flag: &AtomicBool,
) -> CaptureResult<()>
where
    S: std::io::Read + std::io::Write,
{
    loop {
        let message = match ws_stream.read() {
            Ok(message) => message,
            Err(tokio_tungstenite::tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Ok(())
            }
            Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => {
                return Err("websocket connection closed".to_owned())
            }
            Err(error) => return Err(error.to_string()),
        };

        if let Message::Binary(data) = message {
            if let Some((header, _)) = shared::decode_frame(&data) {
                if header.frame_type == shared::FRAME_FORCE_KEYFRAME {
                    force_kf_flag.store(true, Ordering::Relaxed);
                }
            }
        }
    }
}

fn configure_socket_timeouts(
    ws_stream: &WebSocket<MaybeTlsStream<TcpStream>>,
    write_timeout: Duration,
) -> CaptureResult<()> {
    match ws_stream.get_ref() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(Duration::from_millis(1)))
            .and_then(|()| stream.set_write_timeout(Some(write_timeout)))
            .map_err(|error| error.to_string()),
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

fn simple_frame_diff(prev: &[u8], curr: &[u8], sample_step: usize) -> bool {
    if prev.len() != curr.len() || curr.is_empty() {
        return true;
    }

    let sample_step = sample_step.max(1);
    let threshold = (curr.len() / sample_step / 100).max(1);
    let diff_count = prev
        .iter()
        .step_by(sample_step)
        .zip(curr.iter().step_by(sample_step))
        .filter(|(a, b)| a != b)
        .count();

    diff_count > threshold
}
