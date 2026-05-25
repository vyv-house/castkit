use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use crate::room::Room;
use crate::AppState;

pub async fn handle_watch(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(room) = state.rooms.reserve_watcher(&room_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let failed_room_id = room_id.clone();
    let failed_room = room.clone();
    let failed_state = state.clone();

    ws.on_failed_upgrade(move |_error| release_watcher(failed_room_id, failed_room, failed_state))
        .on_upgrade(move |socket| handle_socket(socket, room_id, room, state))
        .into_response()
}

async fn handle_socket(socket: WebSocket, room_id: String, room: Arc<Room>, state: Arc<AppState>) {
    let frames = room.tx.subscribe();
    let (outbound_tx, outbound_rx) = mpsc::channel(256);
    let mut send_task = tokio::spawn(forward_frames(room.clone(), frames, outbound_tx));
    let mut socket_task = tokio::spawn(run_watcher_socket(socket, room.clone(), outbound_rx));

    tokio::select! {
        _ = &mut send_task => {
            socket_task.abort();
        }
        _ = &mut socket_task => {
            send_task.abort();
        }
    }

    release_watcher(room_id, room, state);
}

fn release_watcher(room_id: String, room: Arc<Room>, state: Arc<AppState>) {
    if room.dec_watchers() == 0 && !room.is_producer_connected() {
        state
            .rooms
            .remove_room_if_current(&room_id, &room, room.producer_generation());
    }
}

async fn forward_frames(
    room: Arc<Room>,
    mut frames: broadcast::Receiver<bytes::Bytes>,
    outbound_tx: mpsc::Sender<Message>,
) {
    if let Some(keyframe) = room.get_keyframe() {
        if outbound_tx.send(Message::Binary(keyframe)).await.is_err() {
            return;
        }
    }

    loop {
        match frames.recv().await {
            Ok(frame) => {
                if outbound_tx.send(Message::Binary(frame)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                room.record_watcher_lag(n);
                warn!(room_id = %room.id, skipped = n, "watcher lagged behind broadcast");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn run_watcher_socket(
    mut socket: WebSocket,
    room: Arc<Room>,
    mut outbound_rx: mpsc::Receiver<Message>,
) {
    loop {
        tokio::select! {
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(message) => {
                        if let Err(err) = socket.send(message).await {
                            warn!(room_id = %room.id, error = %err, "watcher websocket send error");
                            break;
                        }
                    }
                    None => break,
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Text(text))) if text == "keyframe" => {
                        if let Err(err) = room.request_force_keyframe() {
                            warn!(room_id = %room.id, error = %err, "failed to request keyframe");
                        } else {
                            room.record_force_keyframe_request();
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(room_id = %room.id, error = %err, "watcher websocket receive error");
                        break;
                    }
                }
            }
        }
    }
}
