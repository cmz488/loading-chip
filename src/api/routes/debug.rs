//! POST /api/debug/start, POST /api/debug/stop — 调试会话管理

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/debug/start", axum::routing::post(debug_start_handler))
        .route("/api/debug/stop", axum::routing::post(debug_stop_handler))
}

#[derive(Debug, Deserialize)]
struct DebugStartRequest {
    elf: String,
    target: String,
    backend: Option<String>,
    interface: Option<String>,
    port: Option<u16>,
}

async fn debug_start_handler(
    State(state): State<AppState>,
    Json(req): Json<DebugStartRequest>,
) -> Json<Value> {
    let backend = req.backend.unwrap_or_else(|| "probe-rs".into());
    let interface = req.interface.unwrap_or_default();
    let port = req.port.unwrap_or(3333);
    {
        let mut b = state.current_board.lock().await;
        *b = Some(req.target.clone());
    }
    {
        let mut be = state.current_backend.lock().await;
        *be = Some(backend.clone());
    }
    Json(json!({
        "status": "started",
        "elf": req.elf, "target": req.target,
        "backend": backend, "interface": interface, "port": port,
    }))
}

async fn debug_stop_handler(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({ "status": "stopped" }))
}
