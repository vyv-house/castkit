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
  --fps 30
```

### 3. Open the viewer

Navigate to `http://localhost:8080` to see live rooms, or go directly to `http://localhost:8080/viewer.html?room=my-room`.

Multi-view: `http://localhost:8080/viewer.html?rooms=room1,room2,room3`

## Wire Protocol

Each WebSocket message is a binary frame:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | Frame type (`0x01` keyframe, `0x02` delta, `0x03` meta, `0x04` force-keyframe) |
| 1 | 4 | Timestamp (ms, big-endian) |
| 5 | 2 | Width (big-endian) |
| 7 | 2 | Height (big-endian) |
| 9 | ... | VP8 payload |

## License

MIT
