//! 应用共享状态 — TUI / CLI / Headless 统一接口
//!
//! `AppState` 是全局单例，所有模式共享同一个实例。
//! 字段用 `Arc` 包裹以支持多模式共享。

use crate::backend::{do_flash, FlashBackend, FlashConfig, FlashResult};
use crate::board::BoardRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

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
    pub run_state: Arc<Mutex<RunState>>,
    pub current_board: Arc<Mutex<Option<String>>>,
    pub current_backend: Arc<Mutex<Option<String>>>,
    pub current_interface: Arc<Mutex<Option<String>>>,
    pub last_result: Arc<Mutex<Option<FlashResult>>>,
}

impl AppState {
    pub fn new(registry: BoardRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            run_state: Arc::new(Mutex::new(RunState::Idle)),
            current_board: Arc::new(Mutex::new(None)),
            current_backend: Arc::new(Mutex::new(None)),
            current_interface: Arc::new(Mutex::new(None)),
            last_result: Arc::new(Mutex::new(None)),
        }
    }

    /// 统一烧录接口 — TUI / CLI / Headless 共用
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
