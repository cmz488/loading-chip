//! GET /api/status — 查询当前烧录状态

use axum::{extract::State, Json, Router};
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/status", axum::routing::get(status_handler))
}

async fn status_handler(State(state): State<AppState>) -> Json<Value> {
    let run_state = state.run_state.lock().await;
    let board = state.current_board.lock().await;
    let backend = state.current_backend.lock().await;
    let last = state.last_result.lock().await;

    Json(json!({
        "state": *run_state,
        "board": *board,
        "backend": *backend,
        "last_result": last.as_ref().map(|r| json!({
            "success": r.success,
            "message": r.message,
            "command": r.command,
            "stdout": r.stdout,
            "stderr": r.stderr,
        })),
    }))
}
