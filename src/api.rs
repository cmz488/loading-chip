//! API 模块 — HTTP REST API + WebSocket（axum 架构）
//!
//! ## 端点
//!
//! | 方法   | 路径                | 说明                         |
//! |--------|---------------------|------------------------------|
//! | GET    | `/api/status`       | 烧录状态查询                 |
//! | GET    | `/api/boards`       | 板子列表                     |
//! | GET    | `/api/boards/{id}`  | 板子详情（含各后端参数）     |
//! | GET    | `/api/detect`       | 芯片自动检测（probe-rs）     |
//! | POST   | `/api/flash`        | 触发烧录                     |
//! | POST   | `/api/debug/start`  | 启动 RTT 调试会话            |
//! | POST   | `/api/debug/stop`   | 停止 RTT 调试会话            |
//! | GET    | `/api/rtt`          | RTT 实时数据流 (WebSocket)   |
//!
//! ## 用法
//!
//! ```bash
//! # 启动 API 服务
//! loading-chip run --api --headless
//!
//! # 状态 & 板子
//! curl http://127.0.0.1:9876/api/status
//! curl http://127.0.0.1:9876/api/boards
//! curl http://127.0.0.1:9876/api/boards/stm32f4
//!
//! # 芯片检测
//! curl http://127.0.0.1:9876/api/detect
//!
//! # 烧录
//! curl -X POST http://127.0.0.1:9876/api/flash \
//!   -H 'Content-Type: application/json' \
//!   -d '{"backend":"probe-rs","board":"esp32s3","elf":"/tmp/fw.elf"}'
//!
//! # RTT 调试（先启动会话，再连接 WebSocket）
//! curl -X POST http://127.0.0.1:9876/api/debug/start \
//!   -H 'Content-Type: application/json' \
//!   -d '{"elf":"/tmp/fw.elf","target":"mspm0g3507"}'
//! websocat ws://127.0.0.1:9876/api/rtt
//!
//! # 停止调试
//! curl -X POST http://127.0.0.1:9876/api/debug/stop
//! ```

pub mod routes;
pub mod server;

#[allow(unused_imports)]
pub use server::{spawn_server, start_server};
