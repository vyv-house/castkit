use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::AppState;
use crate::room::{ProducerCommand, Room};

pub async fn handle_ingest(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if params.get("token") != Some(&state.secret) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, room_id, state))
        .into_response()
}

async fn handle_socket(socket: WebSocket, room_id: String, state: Arc<AppState>) {
    let (room, cmd_rx) = state.rooms.create_room(room_id.clone());
    info!(room_id = %room_id, "producer connected");

    let (outbound_tx, outbound_rx) = mpsc::channel(16);
    let command_task = tokio::spawn(forward_commands(cmd_rx, outbound_tx));
    let socket_task = tokio::spawn(run_producer_socket(socket, room.clone(), outbound_rx));

    let _ = socket_task.await;
    command_task.abort();
    state.rooms.remove_room(&room_id);
    info!(room_id = %room_id, "producer disconnected");
}

async fn run_producer_socket(
    mut socket: WebSocket,
    room: Arc<Room>,
    mut outbound_rx: mpsc::Receiver<Message>,
) {
    loop {
        tokio::select! {
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Binary(frame))) => handle_frame(&room, frame),
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(room_id = %room.id, error = %err, "producer websocket receive error");
                        break;
                    }
                }
            }
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(message) => {
                        if let Err(err) = socket.send(message).await {
                            warn!(room_id = %room.id, error = %err, "producer websocket send error");
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

fn handle_frame(room: &Room, frame: Bytes) {
    let Some((header, _)) = shared::decode_frame(&frame) else {
        warn!(room_id = %room.id, "dropping malformed producer frame");
        return;
    };

    if header.width > 0 && header.height > 0 {
        room.width.store(header.width as usize, Ordering::Relaxed);
        room.height.store(header.height as usize, Ordering::Relaxed);
    }

    if header.frame_type == shared::FRAME_KEYFRAME {
        room.update_keyframe(frame.clone());
    }

    if let Err(err) = room.tx.send(frame) {
        debug!(room_id = %room.id, error = %err, "frame had no active receivers");
    }
}

async fn forward_commands(
    mut cmd_rx: mpsc::Receiver<ProducerCommand>,
    outbound_tx: mpsc::Sender<Message>,
) {
    while let Some(command) = cmd_rx.recv().await {
        match command {
            ProducerCommand::ForceKeyframe => {
                let header = shared::FrameHeader {
                    frame_type: shared::FRAME_FORCE_KEYFRAME,
                    timestamp_ms: 0,
                    width: 0,
                    height: 0,
                };
                let message = Message::Binary(shared::encode_frame(&header, &[]));
                if outbound_tx.send(message).await.is_err() {
                    break;
                }
            }
        }
    }
}
