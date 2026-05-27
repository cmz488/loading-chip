//! API 模块 — HTTP REST API（axum 架构）
//!
//! ## 端点
//!
//! | 方法   | 路径               | 说明             |
//! |--------|--------------------|------------------|
//! | GET    | `/api/status`     | 烧录状态查询     |
//! | GET    | `/api/boards`     | 板子列表         |
//! | GET    | `/api/boards/:id` | 板子详情         |
//! | POST   | `/api/flash`      | 触发烧录         |
//! | GET    | `/api/rtt`        | RTT 实时流(WS)   |
//!
//! ## 用法
//!
//! ```bash
//! # 启动
//! loading-chip run --api --headless
//!
//! # HTTP 调用
//! curl http://127.0.0.1:9876/api/status
//! curl http://127.0.0.1:9876/api/boards
//! curl -X POST http://127.0.0.1:9876/api/flash \
//!   -H 'Content-Type: application/json' \
//!   -d '{"backend":"probe-rs","board":"esp32s3","elf":"/tmp/fw.elf"}'
//!
//! # WebSocket (RTT)
//! websocat ws://127.0.0.1:9876/api/rtt
//! ```

pub mod routes;
pub mod server;

#[allow(unused_imports)]
pub use server::{spawn_server, start_server};
