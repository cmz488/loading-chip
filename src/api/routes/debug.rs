//! POST /api/debug/start, POST /api/debug/stop — RTT 调试会话管理

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::AppState;
use crate::debug::rtt::create_rtt_client;

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
    #[serde(default)]
    #[allow(dead_code)]
    probe: String,
}

async fn debug_start_handler(
    State(state): State<AppState>,
    Json(req): Json<DebugStartRequest>,
) -> Json<Value> {
    let backend = req.backend.unwrap_or_else(|| "probe-rs".into());
    let interface = req.interface.unwrap_or_default();

    // 检查是否已有运行中的会话
    {
        let session = state.rtt_session.lock().await;
        if session.is_some() {
            return Json(json!({ "status": "error", "message": "已有运行中的 RTT 会话，请先调用 /api/debug/stop" }));
        }
    }

    // 使用统一工厂创建 RTT 客户端
    let (tx, _rx) = crossbeam_channel::unbounded();

    let client: Box<dyn crate::debug::rtt::RttClient> = match create_rtt_client(
        &backend,
        &req.target,
        &interface,
        &req.elf,
        3333,
        "", // pyocd_path — API 模式不使用
        tx,
    ) {
        Ok((client, _child)) => client,
        Err(e) => {
            return Json(json!({ "status": "error", "message": e }));
        }
    };

    // 存储会话
    {
        let mut session = state.rtt_session.lock().await;
        *session = Some(client);
    }
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
        "backend": backend, "interface": interface,
    }))
}

async fn debug_stop_handler(State(state): State<AppState>) -> Json<Value> {
    let mut session = state.rtt_session.lock().await;
    match session.take() {
        Some(mut client) => {
            client.stop();
            Json(json!({ "status": "stopped" }))
        }
        None => Json(json!({ "status": "error", "message": "没有运行中的 RTT 会话" })),
    }
}
