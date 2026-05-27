//! GET /api/boards — 板子列表
//! GET /api/boards/{id} — 板子详情

use axum::{
    extract::{Path, State},
    Json, Router,
};
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/boards", axum::routing::get(board_list_handler))
        .route("/api/boards/{id}", axum::routing::get(board_detail_handler))
}

/// 列出所有板子
async fn board_list_handler(State(state): State<AppState>) -> Json<Value> {
    let boards: Vec<Value> = state
        .registry
        .ids()
        .iter()
        .filter_map(|id| {
            state.registry.get(id).map(|info| {
                json!({
                    "id": info.id,
                    "name": info.name,
                    "manufacturer": info.manufacturer,
                    "architecture": info.architecture.to_string(),
                    "interfaces": info.interfaces,
                    "backends": info.supported_backends,
                    "note": info.note,
                })
            })
        })
        .collect();

    Json(json!({ "boards": boards }))
}

/// 获取单板详情（含各后端目标参数）
async fn board_detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    match state.registry.get(&id) {
        Some(info) => {
            let mut backends = serde_json::Map::new();
            for be in &info.supported_backends {
                if let Ok(params) = state.registry.resolve(&id, be) {
                    backends.insert(
                        be.clone(),
                        json!({
                            "target": params.target,
                            "config": params.config,
                            "extra_args": params.extra_args,
                        }),
                    );
                }
            }

            Json(json!({
                "id": info.id,
                "name": info.name,
                "manufacturer": info.manufacturer,
                "architecture": info.architecture.to_string(),
                "interfaces": info.interfaces,
                "backends": backends,
                "note": info.note,
            }))
        }
        None => Json(json!({ "error": format!("未知板子: {}", id) })),
    }
}
