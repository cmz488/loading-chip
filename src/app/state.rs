//! 应用共享状态 — TUI / API / Headless 统一接口
//!
//! `AppState` 是全局单例，所有模式共享同一个实例。
//! axum 要求 handler 共享的状态实现 `Clone`，因此字段用 `Arc` 包裹。

use crate::backend::{do_flash, FlashBackend, FlashConfig, FlashResult};
use crate::board::BoardRegistry;
use crate::chip_detect::DetectedChip;
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
    /// 板子注册表（所有模式共享）
    pub registry: Arc<BoardRegistry>,
    /// 芯片检测结果缓存
    pub detected_chips: Arc<Mutex<Vec<DetectedChip>>>,
    pub run_state: Arc<Mutex<RunState>>,
    pub current_board: Arc<Mutex<Option<String>>>,
    pub current_backend: Arc<Mutex<Option<String>>>,
    pub current_interface: Arc<Mutex<Option<String>>>,
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
            detected_chips: Arc::new(Mutex::new(Vec::new())),
            run_state: Arc::new(Mutex::new(RunState::Idle)),
            current_board: Arc::new(Mutex::new(None)),
            current_backend: Arc::new(Mutex::new(None)),
            current_interface: Arc::new(Mutex::new(None)),
            last_result: Arc::new(Mutex::new(None)),
            rtt_tx,
            rtt_session: Arc::new(Mutex::new(None)),
        }
    }

    /// 运行芯片检测并缓存结果
    pub async fn detect(&self) -> Vec<DetectedChip> {
        let chips = crate::chip_detect::detect_chips();
        let mut cache = self.detected_chips.lock().await;
        *cache = chips.clone();
        chips
    }

    /// 统一烧录接口 — TUI / API / Headless 共用
    pub async fn flash(
        &self,
        backend: &str,
        board_id: &str,
        interface: &str,
        elf_path: &str,
        gdb_port: &str,
        pyocd_path: &str,
        timeout_secs: u64,
    ) -> FlashResult {
        let be = match FlashBackend::from_str(backend) {
            Ok(b) => b,
            Err(e) => {
                let result = FlashResult {
                    success: false, message: e,
                    command: String::new(), stdout: None, stderr: None,
                };
                let mut last = self.last_result.lock().await;
                *last = Some(result.clone());
                let mut s = self.run_state.lock().await;
                *s = RunState::FlashDone;
                return result;
            }
        };

        // 更新状态
        {
            let mut s = self.run_state.lock().await;
            *s = RunState::Flashing;
        }
        {
            let mut b = self.current_board.lock().await;
            *b = Some(board_id.to_string());
        }
        {
            let mut be_state = self.current_backend.lock().await;
            *be_state = Some(be.yaml_key().to_string());
        }
        {
            let mut iface = self.current_interface.lock().await;
            *iface = Some(interface.to_string());
        }

        let config = match FlashConfig::from_registry(
            be, &self.registry, board_id, interface, elf_path,
            gdb_port, pyocd_path, timeout_secs,
        ) {
            Ok(cfg) => cfg,
            Err(msg) => {
                let result = FlashResult {
                    success: false, message: msg,
                    command: String::new(), stdout: None, stderr: None,
                };
                {
                    let mut last = self.last_result.lock().await;
                    *last = Some(result.clone());
                }
                {
                    let mut s = self.run_state.lock().await;
                    *s = RunState::FlashDone;
                }
                return result;
            }
        };

        let result = do_flash(&config);

        {
            let mut last = self.last_result.lock().await;
            *last = Some(result.clone());
        }
        {
            let mut s = self.run_state.lock().await;
            *s = RunState::FlashDone;
        }

        result
    }
}
