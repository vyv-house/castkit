use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arc_swap::ArcSwap;
use bytes::Bytes;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};

const ACTIVE_FRAME_WINDOW_MS: u64 = 5_000;

pub enum ProducerCommand {
    ForceKeyframe,
}

struct ProducerState {
    next_generation: usize,
    connected_generation: usize,
    command_tx: mpsc::Sender<ProducerCommand>,
    last_frame_elapsed_ms: Option<u64>,
}

pub struct Room {
    pub id: String,
    pub tx: broadcast::Sender<Bytes>,
    pub keyframe: ArcSwap<Option<Bytes>>,
    producer_state: Mutex<ProducerState>,
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

    pub fn connect_producer(
        &self,
        id: String,
    ) -> (Arc<Room>, mpsc::Receiver<ProducerCommand>, usize) {
        let (producer_cmd, rx) = mpsc::channel(16);

        match self.inner.entry(id.clone()) {
            Entry::Occupied(entry) => {
                let room = entry.get().clone();
                let generation = room.connect_producer(producer_cmd);
                (room, rx, generation)
            }
            Entry::Vacant(entry) => {
                let (tx, _) = broadcast::channel(256);
                let room = Arc::new(Room {
                    id: id.clone(),
                    tx,
                    keyframe: ArcSwap::from_pointee(None),
                    producer_state: Mutex::new(ProducerState {
                        next_generation: 1,
                        connected_generation: 1,
                        command_tx: producer_cmd,
                        last_frame_elapsed_ms: None,
                    }),
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

                entry.insert(room.clone());
                (room, rx, 1)
            }
        }
    }

    pub fn get_room(&self, id: &str) -> Option<Arc<Room>> {
        self.inner.get(id).map(|room| room.clone())
    }

    pub fn reserve_watcher(&self, id: &str) -> Option<Arc<Room>> {
        self.inner.get(id).map(|room| {
            room.inc_watchers();
            room.clone()
        })
    }

    pub fn remove_room_if_current(
        &self,
        id: &str,
        expected: &Arc<Room>,
        expected_generation: usize,
    ) -> bool {
        self.inner
            .remove_if(id, |_id, room| {
                Arc::ptr_eq(room, expected)
                    && !room.is_producer_connected()
                    && room.producer_generation() == expected_generation
                    && room.watchers.load(Ordering::Relaxed) == 0
            })
            .is_some()
    }

    pub fn list_rooms(&self) -> Vec<shared::RoomInfo> {
        let mut rooms: Vec<_> = self
            .inner
            .iter()
            .map(|entry| entry.value().info())
            .collect();
        rooms.sort_by(|left, right| left.id.cmp(&right.id));
        rooms
    }
}

impl Default for Rooms {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    fn connect_producer(&self, command_tx: mpsc::Sender<ProducerCommand>) -> usize {
        let mut state = self.producer_state.lock().expect("producer state poisoned");
        state.next_generation += 1;
        let generation = state.next_generation;
        state.connected_generation = generation;
        state.command_tx = command_tx;
        state.last_frame_elapsed_ms = None;
        self.width.store(0, Ordering::Relaxed);
        self.height.store(0, Ordering::Relaxed);
        self.last_timestamp_ms.store(0, Ordering::Relaxed);
        self.keyframe.store(Arc::new(None));
        generation
    }

    pub fn mark_producer_disconnected(&self, generation: usize) -> bool {
        let mut state = self.producer_state.lock().expect("producer state poisoned");
        if state.connected_generation == generation {
            state.connected_generation = 0;
            true
        } else {
            false
        }
    }

    pub fn is_current_producer(&self, generation: usize) -> bool {
        self.producer_state
            .lock()
            .expect("producer state poisoned")
            .connected_generation
            == generation
    }

    pub fn is_producer_connected(&self) -> bool {
        self.producer_state
            .lock()
            .expect("producer state poisoned")
            .connected_generation
            != 0
    }

    pub fn producer_generation(&self) -> usize {
        self.producer_state
            .lock()
            .expect("producer state poisoned")
            .next_generation
    }

    pub fn request_force_keyframe(&self) -> Result<(), mpsc::error::TrySendError<ProducerCommand>> {
        let state = self.producer_state.lock().expect("producer state poisoned");
        if state.connected_generation == 0 {
            return Err(mpsc::error::TrySendError::Closed(
                ProducerCommand::ForceKeyframe,
            ));
        }
        state.command_tx.try_send(ProducerCommand::ForceKeyframe)
    }

    pub fn get_keyframe(&self) -> Option<Bytes> {
        self.keyframe.load_full().as_ref().clone()
    }

    pub fn publish_frame(
        &self,
        generation: usize,
        header: &shared::FrameHeader,
        payload_len: usize,
        frame: Bytes,
    ) -> bool {
        let mut state = self.producer_state.lock().expect("producer state poisoned");
        if state.connected_generation != generation {
            return false;
        }

        self.packets_received.fetch_add(1, Ordering::Relaxed);
        self.bytes_received
            .fetch_add(payload_len, Ordering::Relaxed);
        self.last_timestamp_ms
            .store(header.timestamp_ms as usize, Ordering::Relaxed);
        state.last_frame_elapsed_ms = Some(self.created_at.elapsed().as_millis() as u64);

        if header.width > 0 && header.height > 0 {
            self.width.store(header.width as usize, Ordering::Relaxed);
            self.height.store(header.height as usize, Ordering::Relaxed);
        }

        if header.frame_type == shared::FRAME_KEYFRAME {
            self.keyframes_received.fetch_add(1, Ordering::Relaxed);
            self.keyframe.store(Arc::new(Some(frame.clone())));
        }

        let _ = self.tx.send(frame);
        drop(state);
        true
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

    pub fn dec_watchers(&self) -> usize {
        loop {
            let current = self.watchers.load(Ordering::Relaxed);
            if current == 0 {
                return 0;
            }
            let next = current - 1;
            if self
                .watchers
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    pub fn info(&self) -> shared::RoomInfo {
        let elapsed_ms = self.created_at.elapsed().as_millis() as u64;
        let state = self.producer_state.lock().expect("producer state poisoned");
        let last_frame_age_ms = state
            .last_frame_elapsed_ms
            .map(|last_frame_ms| elapsed_ms.saturating_sub(last_frame_ms));
        let producer_connected = state.connected_generation != 0;
        let frame_fresh = last_frame_age_ms.is_none_or(|age_ms| age_ms <= ACTIVE_FRAME_WINDOW_MS);
        let active = producer_connected;
        drop(state);

        shared::RoomInfo {
            id: self.id.clone(),
            width: self.width.load(Ordering::Relaxed) as u16,
            height: self.height.load(Ordering::Relaxed) as u16,
            watchers: self.watchers.load(Ordering::Relaxed),
            active,
            producer_connected,
            frame_fresh,
            last_frame_age_ms,
            metrics: shared::RoomMetrics {
                packets_received: self.packets_received.load(Ordering::Relaxed) as u64,
                keyframes_received: self.keyframes_received.load(Ordering::Relaxed) as u64,
                bytes_received: self.bytes_received.load(Ordering::Relaxed) as u64,
                force_keyframe_requests: self.force_keyframe_requests.load(Ordering::Relaxed)
                    as u64,
                watcher_lag_events: self.watcher_lag_events.load(Ordering::Relaxed) as u64,
                watcher_lagged_frames: self.watcher_lagged_frames.load(Ordering::Relaxed) as u64,
                last_timestamp_ms: self.last_timestamp_ms.load(Ordering::Relaxed) as u32,
                uptime_ms: elapsed_ms,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn room_info_includes_recorded_metrics_and_lifecycle() {
        let rooms = Rooms::new();
        let (room, _rx, generation) = rooms.connect_producer("metrics-room".to_owned());
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

        assert!(room.publish_frame(generation, &keyframe, 1200, Bytes::from_static(b"key")));
        assert!(room.publish_frame(generation, &delta, 800, Bytes::from_static(b"delta")));
        room.record_force_keyframe_request();
        room.record_watcher_lag(3);

        let info = room.info();
        assert!(info.active);
        assert!(info.producer_connected);
        assert!(info.frame_fresh);
        assert!(info.last_frame_age_ms.is_some());
        assert_eq!(info.metrics.packets_received, 2);
        assert_eq!(info.metrics.keyframes_received, 1);
        assert_eq!(info.metrics.bytes_received, 2000);
        assert_eq!(info.metrics.force_keyframe_requests, 1);
        assert_eq!(info.metrics.watcher_lag_events, 1);
        assert_eq!(info.metrics.watcher_lagged_frames, 3);
        assert_eq!(info.metrics.last_timestamp_ms, 84);
        assert!(info.metrics.uptime_ms < 60_000);
    }

    #[test]
    fn active_watchers_keep_room_available_for_reconnect() {
        let rooms = Rooms::new();
        let (room, _rx, generation) = rooms.connect_producer("watched-room".to_owned());
        room.inc_watchers();
        assert!(room.mark_producer_disconnected(generation));

        assert!(!rooms.remove_room_if_current("watched-room", &room, generation));
        assert!(rooms.get_room("watched-room").is_some());

        room.dec_watchers();
        assert!(rooms.remove_room_if_current("watched-room", &room, generation));
        assert!(rooms.get_room("watched-room").is_none());
    }

    #[test]
    fn reconnect_reuses_room_and_stale_disconnect_cannot_remove_it() {
        let rooms = Rooms::new();
        let (room, _first_rx, first_generation) = rooms.connect_producer("room-a".to_owned());
        let (reconnected, _second_rx, second_generation) =
            rooms.connect_producer("room-a".to_owned());
        assert!(Arc::ptr_eq(&room, &reconnected));
        assert!(reconnected.is_producer_connected());
        assert!(!room.mark_producer_disconnected(first_generation));
        assert!(reconnected.is_producer_connected());
        assert!(!rooms.remove_room_if_current("room-a", &room, first_generation));
        assert!(rooms.get_room("room-a").is_some());

        assert!(reconnected.mark_producer_disconnected(second_generation));
        assert!(rooms.remove_room_if_current("room-a", &reconnected, second_generation));
        assert!(rooms.get_room("room-a").is_none());
    }

    #[test]
    fn reconnect_resets_frame_freshness_and_cached_keyframe() {
        let rooms = Rooms::new();
        let (room, _first_rx, first_generation) = rooms.connect_producer("room-a".to_owned());
        let keyframe = shared::FrameHeader {
            frame_type: shared::FRAME_KEYFRAME,
            timestamp_ms: 1,
            width: 640,
            height: 480,
        };
        assert!(room.publish_frame(first_generation, &keyframe, 4, Bytes::from_static(b"oldk")));
        assert!(room.get_keyframe().is_some());
        assert!(room.mark_producer_disconnected(first_generation));

        let (reconnected, _second_rx, _second_generation) =
            rooms.connect_producer("room-a".to_owned());
        assert!(Arc::ptr_eq(&room, &reconnected));
        let info = reconnected.info();
        assert!(info.active);
        assert!(info.frame_fresh);
        assert!(info.last_frame_age_ms.is_none());
        assert_eq!(info.width, 0);
        assert_eq!(info.height, 0);
        assert_eq!(info.metrics.last_timestamp_ms, 0);
        assert!(reconnected.get_keyframe().is_none());
    }

    #[test]
    fn stale_generation_cannot_publish_after_reconnect() {
        let rooms = Rooms::new();
        let (room, _first_rx, first_generation) = rooms.connect_producer("room-a".to_owned());
        let (_same_room, _second_rx, second_generation) =
            rooms.connect_producer("room-a".to_owned());
        let header = shared::FrameHeader {
            frame_type: shared::FRAME_DELTA,
            timestamp_ms: 1,
            width: 640,
            height: 480,
        };

        assert!(!room.publish_frame(first_generation, &header, 4, Bytes::from_static(b"old")));
        assert!(room.publish_frame(second_generation, &header, 4, Bytes::from_static(b"new")));
        assert_eq!(room.info().metrics.packets_received, 1);
    }

    #[test]
    fn reserve_watcher_keeps_cleanup_from_orphaning_watchers() {
        let rooms = Rooms::new();
        let (room, _rx, generation) = rooms.connect_producer("room-a".to_owned());
        assert!(room.mark_producer_disconnected(generation));

        let reserved = rooms.reserve_watcher("room-a").expect("room should exist");
        assert!(Arc::ptr_eq(&room, &reserved));
        assert!(!rooms.remove_room_if_current("room-a", &room, generation));

        assert_eq!(reserved.dec_watchers(), 0);
        assert!(rooms.remove_room_if_current("room-a", &room, generation));
    }

    #[test]
    fn force_keyframe_request_uses_current_producer_sender() {
        let rooms = Rooms::new();
        let (room, first_rx, first_generation) = rooms.connect_producer("room-a".to_owned());
        drop(first_rx);
        assert!(room.mark_producer_disconnected(first_generation));
        let (_room, mut second_rx, _second_generation) =
            rooms.connect_producer("room-a".to_owned());

        room.request_force_keyframe()
            .expect("current sender should accept command");
        assert!(second_rx.try_recv().is_ok());
    }

    #[test]
    fn simultaneous_first_connects_share_one_room() {
        let rooms = Arc::new(Rooms::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let rooms = rooms.clone();
            handles.push(thread::spawn(move || {
                let (room, _rx, generation) = rooms.connect_producer("room-a".to_owned());
                (room, generation)
            }));
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.join().expect("connect thread should finish"));
        }

        let first = results[0].0.clone();
        for (room, _) in &results {
            assert!(Arc::ptr_eq(&first, room));
        }
        let max_generation = results
            .iter()
            .map(|(_, generation)| *generation)
            .max()
            .unwrap();
        assert_eq!(max_generation, results.len());
        assert!(rooms.get_room("room-a").is_some());
    }
}
