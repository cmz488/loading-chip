//! GET /api/detect — 芯片检测

use axum::{extract::State, Json, Router};
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/detect", axum::routing::get(detect_handler))
}

async fn detect_handler(State(state): State<AppState>) -> Json<Value> {
    let chips: Vec<Value> = state.detect().await
        .into_iter()
        .map(|d| json!({
            "probe_name": d.probe_name,
            "chip_name": d.chip_name,
            "suggested_interface": d.suggested_interface,
        }))
        .collect();
    Json(json!({ "detected": chips }))
}
