use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use crate::AppState;
use crate::room::{ProducerCommand, Room};

pub async fn handle_watch(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(room) = state.rooms.get_room(&room_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    ws.on_upgrade(move |socket| handle_socket(socket, room))
        .into_response()
}

async fn handle_socket(socket: WebSocket, room: Arc<Room>) {
    room.inc_watchers();

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

    room.dec_watchers();
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
                        if let Err(err) = room.producer_cmd.send(ProducerCommand::ForceKeyframe).await {
                            warn!(room_id = %room.id, error = %err, "failed to request keyframe");
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
