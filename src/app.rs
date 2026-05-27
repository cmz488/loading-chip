//! 应用状态模块 — axum 共享状态
//!
//! `AppState` 持有板子注册表、烧录状态、RTT 广播器等，
//! 通过 axum 的 `State` extractor 注入到各路由处理器。

pub mod state;
