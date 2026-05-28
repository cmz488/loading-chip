//! POST /api/flash — 触发烧录（统一接口）

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/flash", axum::routing::post(flash_handler))
}

#[derive(Debug, Deserialize)]
struct FlashRequest {
    backend: Option<String>,
    board: Option<String>,
    elf: Option<String>,
    interface: Option<String>,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default)]
    gdb_port: String,
    #[serde(default)]
    pyocd_path: String,
}

fn default_timeout() -> u64 { 60 }

async fn flash_handler(
    State(state): State<AppState>,
    Json(req): Json<FlashRequest>,
) -> Json<Value> {
    let backend = req.backend.unwrap_or_else(|| "probe-rs".into());
    let board = match req.board {
        Some(b) => b,
        None => return Json(json!({ "success": false, "message": "缺少 board 参数" })),
    };
    let elf = match req.elf {
        Some(e) => e,
        None => return Json(json!({ "success": false, "message": "缺少 elf 参数" })),
    };
    let interface = req.interface.unwrap_or_default();
    let gdb_port = if req.gdb_port.is_empty() { "3333".into() } else { req.gdb_port };

    // 使用统一 flash 接口（与 TUI / Headless / CLI 共享同一逻辑）
    let result = state.flash(&backend, &board, &interface, &elf, &gdb_port, &req.pyocd_path, req.timeout).await;

    Json(json!({
        "success": result.success,
        "message": result.message,
        "command": result.command,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }))
}
