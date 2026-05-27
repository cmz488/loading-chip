//! POST /api/flash — 触发烧录

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::{AppState, RunState};
use crate::backend::{do_flash, FlashBackend, FlashConfig};

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

fn default_timeout() -> u64 {
    60
}

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

    let be = FlashBackend::from_str(&backend);
    let backend_name = be.yaml_key();

    // 解析板子+后端兼容性
    let params = match state.registry.resolve(&board, backend_name) {
        Ok(p) => p,
        Err(e) => return Json(json!({ "success": false, "message": e })),
    };

    // 更新状态
    {
        let mut s = state.run_state.lock().await;
        *s = RunState::Flashing;
    }
    {
        let mut b = state.current_board.lock().await;
        *b = Some(board.clone());
    }
    {
        let mut be_state = state.current_backend.lock().await;
        *be_state = Some(backend_name.to_string());
    }

    // 执行烧录（阻塞操作，用 spawn_blocking 避免阻塞 async runtime）
    let registry_id = board.clone();
    let result = tokio::task::spawn_blocking(move || {
        let config = FlashConfig {
            backend: be,
            interface: req.interface.unwrap_or_default(),
            target: params.target,
            elf_path: elf,
            gdb_port: if req.gdb_port.is_empty() {
                "3333".into()
            } else {
                req.gdb_port
            },
            pyocd_path: req.pyocd_path,
            timeout_secs: req.timeout,
            board_config: params.config,
            board_extra_args: params.extra_args,
            board_id: registry_id,
        };
        do_flash(&config)
    })
    .await
    .unwrap_or_else(|e| {
        crate::backend::FlashResult {
            success: false,
            message: format!("烧录任务失败: {}", e),
            command: String::new(),
            stdout: None,
            stderr: None,
        }
    });

    // 更新结果
    {
        let mut s = state.run_state.lock().await;
        *s = RunState::FlashDone;
    }
    {
        let mut last = state.last_result.lock().await;
        *last = Some(result.clone());
    }

    Json(json!({
        "success": result.success,
        "message": result.message,
        "command": result.command,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }))
}
