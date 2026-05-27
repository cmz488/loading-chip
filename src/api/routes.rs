//! API 路由模块 — 路由组装
//!
//! 将各子模块的路由合并为统一的 axum Router。

pub mod board;
pub mod flash;
pub mod rtt;
pub mod status;

use axum::Router;
use crate::app::state::AppState;

/// 构建所有 API 路由
pub fn api_router() -> Router<AppState> {
    Router::new()
        .merge(status::routes())
        .merge(board::routes())
        .merge(flash::routes())
        .merge(rtt::routes())
}
