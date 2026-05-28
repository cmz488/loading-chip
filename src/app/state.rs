//! 应用共享状态
//!
//! axum 要求所有 handler 共享的状态实现 `Clone`，
//! 因此内部字段用 `Arc` 包裹。

use crate::backend::FlashResult;
use crate::board::BoardRegistry;
use crate::debug::rtt::{RttClient, RttOutput};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

// ============================================================================
// 运行状态
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Idle,
    Flashing,
    FlashDone,
}

// ============================================================================
// AppState
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<BoardRegistry>,
    pub run_state: Arc<Mutex<RunState>>,
    pub current_board: Arc<Mutex<Option<String>>>,
    pub current_backend: Arc<Mutex<Option<String>>>,
    pub last_result: Arc<Mutex<Option<FlashResult>>>,
    /// RTT 广播器（TUI / API 共享）
    pub rtt_tx: broadcast::Sender<RttOutput>,
    /// 活跃的 RTT 会话（API 模式管理）
    pub rtt_session: Arc<Mutex<Option<Box<dyn RttClient>>>>,
}

impl AppState {
    pub fn new(registry: BoardRegistry) -> Self {
        let (rtt_tx, _) = broadcast::channel(256);
        Self {
            registry: Arc::new(registry),
            run_state: Arc::new(Mutex::new(RunState::Idle)),
            current_board: Arc::new(Mutex::new(None)),
            current_backend: Arc::new(Mutex::new(None)),
            last_result: Arc::new(Mutex::new(None)),
            rtt_tx,
            rtt_session: Arc::new(Mutex::new(None)),
        }
    }
}
