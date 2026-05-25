use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::AppState;

pub async fn list_rooms(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.rooms.list_rooms())
}

pub async fn room_detail(
    Path(room_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.rooms.get_room(&room_id) {
        Some(room) => Json(room.info()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
