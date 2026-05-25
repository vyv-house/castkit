# Transport and codec decision

Status: keep the current VP8-over-WebSocket relay for LAN screen viewing. Defer a transport or codec redesign until LAN telemetry shows the current path missing the target envelope below.

## Decision drivers

Castkit is intentionally scoped to one-way, low-latency LAN viewing: no audio, recording, remote control, accounts, internet relay, or NAT traversal. That scope makes simplicity and observable latency more valuable than adding a peer-to-peer signaling stack or a second codec before the current bottleneck is proven.

The current path is:

```text
macOS capture -> BGRA to I420 -> VP8 realtime encode -> binary WebSocket relay -> WebCodecs VP8 decode
```

Recent telemetry and policy hooks expose the measurements needed to evaluate the path before redesigning it:

| Layer | Existing signal | Redesign trigger |
|-------|-----------------|------------------|
| Producer CPU | `pipeline_busy_pct`, `avg_convert_ms`, `avg_encode_ms` | Sustained encode+convert time consumes most of the frame budget at the target FPS after lowering resolution/quality |
| Producer backpressure | `dropped_capture_frames`, `stale_frames_dropped`, `avg_send_ms` | Dropped/stale frames continue while `avg_encode_ms` is healthy, indicating send or relay pressure instead of encoder pressure |
| Gateway relay | `watcher_lag_events`, `watcher_lagged_frames`, `bytes_received`, `packets_received` | Watchers lag while producer send time and viewer decode queue are healthy, indicating relay fanout pressure |
| Viewer decode | decoded FPS, decode queue depth, dropped frames, keyframe recovery time, estimated p95 playout delay | Viewer delay or recovery misses target while producer/gateway telemetry remains healthy |

## Protocol-overhead comparison

The shared wire protocol adds a 9-byte Castkit header to each encoded frame. At 30 FPS, that is 270 bytes/s, or about 2.16 kbit/s before WebSocket framing. Compared with the current VP8 bitrate policy, protocol overhead is not a meaningful reason to redesign transport:

| Target | VP8 policy bitrate | Castkit header overhead at 30 FPS | Header share |
|--------|--------------------|------------------------------------|--------------|
| 720p low-latency (`quality=0.75`) | ~750 kbit/s | ~2.16 kbit/s | ~0.29% |
| 1080p low-latency (`quality=0.75`) | ~1,555 kbit/s | ~2.16 kbit/s | ~0.14% |
| 1080p balanced (`quality=1.0`) | ~2,073 kbit/s | ~2.16 kbit/s | ~0.10% |

The table uses the encoder bitrate policy in `crates/capture/src/encoder.rs` and the frame header size in `crates/shared/src/lib.rs`. It shows that WebRTC/WebTransport would not materially improve per-frame payload overhead for the current one-way LAN target. A redesign should therefore be justified by measured congestion control, fanout, NAT traversal, or hardware codec needs rather than by framing overhead.

## Alternatives considered

### Keep VP8 over binary WebSocket

Pros:
- Already matches the one-way relay model and browser WebCodecs VP8 decode path.
- Keeps gateway behavior debuggable with simple WebSocket ingest/watch endpoints.
- Avoids signaling, ICE, STUN/TURN, SDP, and peer lifecycle complexity that mostly serves NAT traversal and audio/video conferencing requirements outside the current scope.
- Existing keyframe cache and force-keyframe control path support late join and recovery without renegotiation.

Cons:
- No built-in congestion control beyond application-level queues, frame drops, and operator policy.
- Relay fanout remains gateway-bound.
- Does not provide peer-to-peer internet traversal if the product scope changes.

Decision: keep.

### WebRTC transport

Pros:
- Mature congestion control, jitter buffering, and optional peer-to-peer paths.
- Natural fit if audio, NAT traversal, or internet relay become product requirements.

Cons:
- Requires signaling and peer/session state not needed for the current LAN relay.
- Pushes the project toward non-goals: NAT traversal, internet relay, audio/video conferencing semantics, and more complex multi-view lifecycle.
- Adds a second latency stack before telemetry proves the relay path is the bottleneck.

Decision: defer until NAT traversal, audio, or measured relay congestion becomes a requirement.

### WebTransport or QUIC

Pros:
- Lower-level transport controls than WebSocket and potential future fit for datagram-style media.

Cons:
- Requires HTTP/3/QUIC deployment complexity and still needs browser support and application-level media recovery.
- Does not solve codec CPU pressure.
- Current per-frame overhead measurement is too small to justify the operational cost.

Decision: defer.

### H.264 / VideoToolbox encode

Pros:
- Strong hardware acceleration story on macOS and broad browser decode support.
- Could reduce producer CPU if VP8 encode time becomes the bottleneck.

Cons:
- Adds platform-specific encode code and codec negotiation.
- Licensing/distribution and encoder-configuration complexity are higher than VP8/libvpx.
- Cross-platform producer is currently a non-goal, so this should be a measured macOS optimization rather than a default redesign.

Decision: prototype only if `avg_encode_ms` or `pipeline_busy_pct` show VP8 encode pressure after using low-latency/p720 policy.

### VP9 or AV1

Pros:
- Better compression efficiency at similar visual quality.

Cons:
- Higher realtime encode CPU cost, especially for screen sharing at low latency.
- Benefits bandwidth more than latency for LAN use; current policy already reaches low-Mbit bitrates.

Decision: defer unless bandwidth is the measured bottleneck and producer CPU has clear headroom.

## Go/no-go thresholds

Keep VP8/WebSocket when a 60-second LAN run with the recommended low-latency policy shows:

- decoded FPS tracks target FPS closely,
- viewer p95 estimated playout delay stays under 200 ms,
- producer `pipeline_busy_pct` leaves meaningful CPU headroom,
- `avg_encode_ms` remains well below the frame interval,
- dropped/stale frames are rare outside intentional load tests,
- gateway watcher lag counters stay near zero.

Start a redesign/prototype only when telemetry isolates a persistent bottleneck:

- **Codec prototype:** `avg_encode_ms` or `pipeline_busy_pct` dominates frame time after lowering resolution/quality.
- **Transport prototype:** `avg_send_ms`, watcher lag, or viewer delay remains high while encode/decode metrics are healthy.
- **Architecture expansion:** product scope adds internet traversal, audio, recording, or multi-site relay.

## Conclusion

No transport or codec rewrite is justified by the current one-way LAN scope or by protocol overhead. The safer path is to keep VP8 over binary WebSocket, use the producer policy flags to tune latency/quality, and collect the telemetry above before introducing WebRTC, QUIC/WebTransport, H.264, VP9, or AV1.
