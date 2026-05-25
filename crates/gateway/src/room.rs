use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};

pub enum ProducerCommand {
    ForceKeyframe,
}

pub struct Room {
    pub id: String,
    pub tx: broadcast::Sender<Bytes>,
    pub keyframe: ArcSwap<Option<Bytes>>,
    pub producer_cmd: mpsc::Sender<ProducerCommand>,
    pub width: AtomicUsize,
    pub height: AtomicUsize,
    pub watchers: AtomicUsize,
    pub packets_received: AtomicUsize,
    pub keyframes_received: AtomicUsize,
    pub bytes_received: AtomicUsize,
    pub force_keyframe_requests: AtomicUsize,
    pub watcher_lag_events: AtomicUsize,
    pub watcher_lagged_frames: AtomicUsize,
    pub last_timestamp_ms: AtomicUsize,
    pub created_at: Instant,
}

pub struct Rooms {
    inner: DashMap<String, Arc<Room>>,
}

impl Rooms {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    pub fn create_room(&self, id: String) -> (Arc<Room>, mpsc::Receiver<ProducerCommand>) {
        let (tx, _) = broadcast::channel(256);
        let (producer_cmd, rx) = mpsc::channel(16);
        let room = Arc::new(Room {
            id: id.clone(),
            tx,
            keyframe: ArcSwap::from_pointee(None),
            producer_cmd,
            width: AtomicUsize::new(0),
            height: AtomicUsize::new(0),
            watchers: AtomicUsize::new(0),
            packets_received: AtomicUsize::new(0),
            keyframes_received: AtomicUsize::new(0),
            bytes_received: AtomicUsize::new(0),
            force_keyframe_requests: AtomicUsize::new(0),
            watcher_lag_events: AtomicUsize::new(0),
            watcher_lagged_frames: AtomicUsize::new(0),
            last_timestamp_ms: AtomicUsize::new(0),
            created_at: Instant::now(),
        });

        self.inner.insert(id, room.clone());
        (room, rx)
    }

    pub fn get_room(&self, id: &str) -> Option<Arc<Room>> {
        self.inner.get(id).map(|room| room.clone())
    }

    pub fn remove_room(&self, id: &str) {
        self.inner.remove(id);
    }

    pub fn list_rooms(&self) -> Vec<shared::RoomInfo> {
        self.inner
            .iter()
            .map(|entry| entry.value().info())
            .collect()
    }
}

impl Default for Rooms {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    pub fn update_keyframe(&self, frame: Bytes) {
        self.keyframe.store(Arc::new(Some(frame)));
    }

    pub fn get_keyframe(&self) -> Option<Bytes> {
        self.keyframe.load_full().as_ref().clone()
    }

    pub fn record_frame(&self, header: &shared::FrameHeader, payload_len: usize) {
        self.packets_received.fetch_add(1, Ordering::Relaxed);
        self.bytes_received
            .fetch_add(payload_len, Ordering::Relaxed);
        self.last_timestamp_ms
            .store(header.timestamp_ms as usize, Ordering::Relaxed);
        if header.frame_type == shared::FRAME_KEYFRAME {
            self.keyframes_received.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_force_keyframe_request(&self) {
        self.force_keyframe_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_watcher_lag(&self, skipped: u64) {
        self.watcher_lag_events.fetch_add(1, Ordering::Relaxed);
        self.watcher_lagged_frames
            .fetch_add(skipped as usize, Ordering::Relaxed);
    }

    pub fn inc_watchers(&self) {
        self.watchers.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_watchers(&self) {
        self.watchers.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn info(&self) -> shared::RoomInfo {
        shared::RoomInfo {
            id: self.id.clone(),
            width: self.width.load(Ordering::Relaxed) as u16,
            height: self.height.load(Ordering::Relaxed) as u16,
            watchers: self.watchers.load(Ordering::Relaxed),
            active: true,
            metrics: shared::RoomMetrics {
                packets_received: self.packets_received.load(Ordering::Relaxed) as u64,
                keyframes_received: self.keyframes_received.load(Ordering::Relaxed) as u64,
                bytes_received: self.bytes_received.load(Ordering::Relaxed) as u64,
                force_keyframe_requests: self.force_keyframe_requests.load(Ordering::Relaxed)
                    as u64,
                watcher_lag_events: self.watcher_lag_events.load(Ordering::Relaxed) as u64,
                watcher_lagged_frames: self.watcher_lagged_frames.load(Ordering::Relaxed) as u64,
                last_timestamp_ms: self.last_timestamp_ms.load(Ordering::Relaxed) as u32,
                uptime_ms: self.created_at.elapsed().as_millis() as u64,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_info_includes_recorded_metrics() {
        let rooms = Rooms::new();
        let (room, _rx) = rooms.create_room("metrics-room".to_owned());
        let keyframe = shared::FrameHeader {
            frame_type: shared::FRAME_KEYFRAME,
            timestamp_ms: 42,
            width: 1920,
            height: 1080,
        };
        let delta = shared::FrameHeader {
            frame_type: shared::FRAME_DELTA,
            timestamp_ms: 84,
            width: 1920,
            height: 1080,
        };

        room.record_frame(&keyframe, 1200);
        room.record_frame(&delta, 800);
        room.record_force_keyframe_request();
        room.record_watcher_lag(3);

        let info = room.info();
        assert_eq!(info.metrics.packets_received, 2);
        assert_eq!(info.metrics.keyframes_received, 1);
        assert_eq!(info.metrics.bytes_received, 2000);
        assert_eq!(info.metrics.force_keyframe_requests, 1);
        assert_eq!(info.metrics.watcher_lag_events, 1);
        assert_eq!(info.metrics.watcher_lagged_frames, 3);
        assert_eq!(info.metrics.last_timestamp_ms, 84);
        assert!(info.metrics.uptime_ms < 60_000);
    }
}
