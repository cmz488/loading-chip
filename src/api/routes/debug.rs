//! POST /api/debug/start, POST /api/debug/stop — RTT 调试会话管理

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::AppState;
use crate::debug::rtt::{ProbeRsRtt, RttBackend, RttConfig};

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

    // 根据后端类型创建 RTT 配置
    let (tx, _rx) = crossbeam_channel::unbounded();
    let rtt_backend = RttBackend::from_str(&backend);
    let config = RttConfig {
        backend: rtt_backend,
        chip: req.target.clone(),
        probe: req.probe,
        telnet_port: 3333,
        elf_path: Some(req.elf.clone()),
        broadcast: Some(state.rtt_tx.clone()),
    };

    // 启动 RTT 客户端
    let client: Box<dyn crate::debug::rtt::RttClient> = match rtt_backend {
        RttBackend::ProbeRs => {
            match ProbeRsRtt::spawn(&config, tx) {
                Ok(c) => Box::new(c),
                Err(e) => return Json(json!({ "status": "error", "message": format!("启动 probe-rs RTT 失败: {}", e) })),
            }
        }
        RttBackend::OpenOcd => {
            match crate::debug::rtt::spawn_openocd_rtt(4444, tx) {
                Ok(c) => Box::new(c),
                Err(e) => return Json(json!({ "status": "error", "message": format!("启动 OpenOCD RTT 失败: {}", e) })),
            }
        }
        RttBackend::None => {
            return Json(json!({ "status": "error", "message": "不支持此后端类型的 RTT" }));
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
