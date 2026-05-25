# Castkit

Low-latency screen sharing over WebSocket. Captures the screen via macOS native APIs, encodes to VP8, and streams to browser viewers through a relay gateway.

## Architecture

```
Producer (macOS)          Gateway (server)           Viewer (browser)
+-----------------+      +------------------+      +------------------+
| CGDisplayStream  | ---> | Axum WebSocket   | ---> | WebCodecs VP8    |
| libyuv BGRA->YUV|  WS  | broadcast::channel|  WS  | Canvas 2D render |
| libvpx VP8 encode|      | Room management  |      | Jitter buffer    |
+-----------------+      +------------------+      +------------------+
```

### Crates

| Crate | Description |
|-------|-------------|
| `shared` | Wire protocol — 9-byte frame header, encode/decode, common types |
| `capture` | macOS screen capture (CGDisplayStream + IOSurface FFI), BGRA→I420 via libyuv, VP8 encoding via libvpx |
| `gateway` | Axum-based relay server — WebSocket ingest/watch, room management, static file serving |
| `producer` | CLI that captures the screen and streams encoded frames to the gateway |

### Web Viewer

Browser-based viewer using WebCodecs `VideoDecoder` for hardware-accelerated VP8 decoding. Supports single-room and multi-room grid views.

## Prerequisites

- macOS (Apple Silicon or Intel)
- Rust toolchain (stable)
- `libvpx` and `libyuv` — via Nix, vcpkg, or Homebrew
- Screen recording permission granted to the terminal

### Installing native dependencies

**Nix (recommended):**
```bash
# Set paths or use pkg-config; the build script auto-detects via pkg-config
nix-shell -p libvpx libyuv pkg-config
```

**Homebrew:**
```bash
brew install libvpx libyuv
```

**vcpkg:**
```bash
export VCPKG_ROOT=/path/to/vcpkg
vcpkg install libvpx libyuv
```

## Build

```bash
cargo build --release
```

## Usage

### 1. Start the gateway

```bash
cargo run --release -p gateway -- --port 8080 --secret changeme --web-dir web
```

### 2. Start the producer

```bash
cargo run --release -p producer -- \
  --room-id my-room \
  --gateway-url ws://127.0.0.1:8080 \
  --secret changeme \
  --profile low-latency \
  --resolution p720 \
  --fps 30
```

Useful producer policy flags:

| Flag | Meaning |
|------|---------|
| `--profile low-latency\|balanced\|quality` | Applies default quality, encoder speed, keyframe interval, and queue depths for the latency/detail tradeoff |
| `--resolution source\|p720\|p1080` | Captures at source size or a 16:9 preset; `--width`/`--height` override the preset |
| `--quality 0.1..2.0` | VP8 bitrate/quantizer quality multiplier; lower favors latency and bandwidth |
| `--keyframe-interval N` | Maximum encoded-frame interval between automatic keyframes |
| `--capture-queue-depth N` | Bounded producer-side frame queue; stale queued frames are dropped before encode so the newest frame wins |
| `--stream-queue-length N` | Native capture stream queue depth, clamped to the CoreGraphics-supported range |
| `--write-timeout-ms N` | WebSocket write timeout to prevent send stalls from indefinitely blocking capture/encode |
| `--stretch` | Stretch to the requested resolution instead of preserving aspect ratio with letterboxing |

For Discord-like LAN viewing, start with `--profile low-latency --resolution p720 --fps 30`, then raise `--resolution`, `--quality`, or queue depths only if telemetry shows spare encode/send headroom.

### 3. Open the viewer

Navigate to `http://localhost:8080` to see live rooms, or go directly to `http://localhost:8080/viewer.html?room=my-room`.

Multi-view: `http://localhost:8080/viewer.html?rooms=room1,room2,room3`


## Telemetry and benchmarking hooks

The system exposes lightweight counters that are useful for LAN latency and throughput tuning without adding remote-control, recording, audio, account, or relay features.

See [`docs/transport-codec-decision.md`](docs/transport-codec-decision.md) for the current transport/codec decision, protocol-overhead comparison, and redesign thresholds.

### Producer telemetry

The producer logs one `producer telemetry` event per second. Useful fields:

| Field | Meaning |
|-------|---------|
| `fps` | Captured frames per second over the last interval |
| `kbps` | WebSocket payload throughput sent by the producer |
| `pipeline_busy_pct` | Conversion, encode, and send time as a percentage of wall-clock time; a producer CPU-pressure proxy |
| `dropped_capture_frames` | Captured frames dropped because the encoder/send loop was backpressured |
| `stale_frames_dropped` | Older queued producer frames discarded before encode so the viewer receives the freshest frame |
| `avg_convert_ms` | Average BGRA→YUV conversion time per captured frame |
| `avg_encode_ms` | Average VP8 encode time per captured frame |
| `avg_send_ms` | Average WebSocket send time per packet |
| `keyframes_sent` | Keyframes produced in the interval, including recovery requests |

Run with info logging enabled while testing a room:

```bash
RUST_LOG=producer=info cargo run --release -p producer -- \
  --room-id my-room \
  --gateway-url ws://127.0.0.1:8080 \
  --secret changeme \
  --fps 30
```

### Room lifecycle metadata

`GET /api/rooms` keeps recently disconnected rooms visible briefly so operators can see streams drop instead of losing tiles immediately. The viewer also releases non-pinned inactive streams after its grace window so backend retention can expire. Each room also includes:

| Field | Meaning |
|-------|---------|
| `active` | Producer WebSocket is currently connected and the room is watchable |
| `producer_connected` | Producer WebSocket is currently connected |
| `frame_fresh` | Latest frame is recent, or the producer is still waiting for its first frame |
| `last_frame_age_ms` | Wall-clock age of the latest received frame, or `null` before the first frame |

### Gateway room metrics

`GET /api/rooms` includes cumulative per-room `metrics`:

| Field | Meaning |
|-------|---------|
| `packets_received` | Producer packets accepted by the gateway |
| `keyframes_received` | Keyframes accepted by the gateway |
| `bytes_received` | Encoded video bytes received, excluding the protocol header |
| `force_keyframe_requests` | Viewer recovery requests forwarded toward the producer |
| `watcher_lag_events` | Broadcast lag events observed by viewer sockets |
| `watcher_lagged_frames` | Total skipped broadcast frames across lag events |
| `last_timestamp_ms` | Latest producer timestamp observed by the gateway |
| `uptime_ms` | Room lifetime used for average packet-rate calculations |

The room directory displays the most important counters so an operator can spot low FPS, low throughput, lag, and keyframe-recovery churn before opening a stream.

### Viewer telemetry overlay

The multi-room viewer shows decoded FPS plus a per-stream metrics pill with received KiB/s, current and rolling p95 estimated local playout delay, decode queue depth, keyframe recovery time, and dropped frames. The delay estimate is a client-side clock-offset heuristic, intended for relative benchmarking across changes rather than absolute glass-to-glass measurement. The displayed drop total includes both pre-keyframe discard and decode-queue backpressure discard; the second number isolates decode-queue drops.

For a baseline LAN benchmark, run the gateway and one or more producers for at least 60 seconds, then record producer logs, `/api/rooms`, and the viewer overlay while targeting 1080p/30fps and p95 estimated viewer delay under 200 ms; use the heuristic as a regression signal, not proof of absolute glass-to-glass latency.

## Wire Protocol

Each producer-to-gateway and gateway-to-viewer WebSocket video message is a binary frame:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | Frame type (`0x01` keyframe, `0x02` delta, `0x03` meta, `0x04` force-keyframe) |
| 1 | 4 | Timestamp (ms, big-endian) |
| 5 | 2 | Width (big-endian) |
| 7 | 2 | Height (big-endian) |
| 9 | ... | VP8 payload |

Viewer-to-gateway control messages currently use a text WebSocket command:

| Message | Meaning |
|---------|---------|
| `keyframe` | Ask the gateway to forward a force-keyframe request to the producer. |

The gateway forwards that request to the producer as an empty binary frame with type `0x04`.

## License

MIT
