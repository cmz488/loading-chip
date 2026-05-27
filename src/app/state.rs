//! 应用共享状态
//!
//! axum 要求所有 handler 共享的状态实现 `Clone`，
//! 因此内部字段用 `Arc` 包裹。

use crate::backend::FlashResult;
use crate::board::BoardRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

// ============================================================================
// 运行状态
// ============================================================================

/// 程序运行状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Idle,
    Flashing,
    FlashDone,
}

// ============================================================================
// RTT 事件
// ============================================================================

/// RTT 输出事件（通过 broadcast channel 推送到 WebSocket）
#[derive(Debug, Clone, Serialize)]
pub struct RttEvent {
    pub channel: u8,
    pub data: String,
}

// ============================================================================
// AppState
// ============================================================================

/// 应用全局状态（axum State extractor）
///
/// 所有字段用 `Arc<Mutex<>>` 或 `broadcast::Sender` 实现共享。
#[derive(Clone)]
pub struct AppState {
    /// 板子注册表（只读，无需 Mutex）
    pub registry: Arc<BoardRegistry>,
    /// 当前运行状态
    pub run_state: Arc<Mutex<RunState>>,
    /// 当前板子 ID
    pub current_board: Arc<Mutex<Option<String>>>,
    /// 当前后端
    pub current_backend: Arc<Mutex<Option<String>>>,
    /// 最后一次烧录结果
    pub last_result: Arc<Mutex<Option<FlashResult>>>,
    /// RTT 广播器（订阅者通过 subscribe() 获取接收端）
    pub rtt_tx: broadcast::Sender<RttEvent>,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(registry: BoardRegistry) -> Self {
        let (rtt_tx, _) = broadcast::channel(256);
        Self {
            registry: Arc::new(registry),
            run_state: Arc::new(Mutex::new(RunState::Idle)),
            current_board: Arc::new(Mutex::new(None)),
            current_backend: Arc::new(Mutex::new(None)),
            last_result: Arc::new(Mutex::new(None)),
            rtt_tx,
        }
    }

    /// 推送 RTT 数据到所有 WebSocket 订阅者（预留 TUI 集成）
    #[allow(dead_code)]
    pub fn push_rtt(&self, channel: u8, data: &str) {
        let _ = self.rtt_tx.send(RttEvent {
            channel,
            data: data.to_string(),
        });
    }

    /// 推送 RTT 数据（拥有 data，预留 TUI 集成）
    #[allow(dead_code)]
    pub fn push_rtt_owned(&self, channel: u8, data: String) {
        let _ = self.rtt_tx.send(RttEvent { channel, data });
    }
}
