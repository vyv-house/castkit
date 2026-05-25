use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
        }
    }
}
