//! 应用共享状态 — TUI / CLI / Headless 统一接口
//!
//! `AppState` 是全局单例，所有模式共享同一个实例。

use crate::backend::{do_flash, FlashBackend, FlashConfig, FlashResult};
use crate::board::BoardRegistry;
use std::sync::Arc;

// ============================================================================
// AppState
// ============================================================================

/// 全局应用状态，持有板子注册表
#[derive(Clone)]
pub struct AppState {
    /// 板子注册表（所有模式共享）
    pub registry: Arc<BoardRegistry>,
}

impl AppState {
    pub fn new(registry: BoardRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// 统一烧录接口 — CLI / Headless 共用
    ///
    /// TUI 有自己的 `App::do_flash()` 直接调用 `do_flash()`，
    /// 因为它管理独立的 UI 参数状态。
    pub fn flash(
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
                return FlashResult {
                    success: false,
                    message: e,
                    command: String::new(),
                    stdout: None,
                    stderr: None,
                };
            }
        };

        let config = match FlashConfig::from_registry(
            be, &self.registry, board_id, interface, elf_path,
            gdb_port, pyocd_path, timeout_secs,
        ) {
            Ok(cfg) => cfg,
            Err(msg) => {
                return FlashResult {
                    success: false,
                    message: msg,
                    command: String::new(),
                    stdout: None,
                    stderr: None,
                };
            }
        };

        do_flash(&config)
    }
}
