//! 烧录后端抽象层
//!
//! 通过 `Backend` trait 统一四种烧录后端的接口：
//! - `GdbBackend` — arm-none-eabi-gdb（批量模式连接 GDB Server）
//! - `OpenOcdBackend` — OpenOCD（直接调用，无需外部 GDB Server）
//! - `ProbeRsBackend` — probe-rs（Rust 原生，零配置，速度最快）
//! - `PyOcdBackend` — pyOCD（Python 工具，CMSIS-Pack 生态）
//!
//! 调度逻辑通过 `FlashBackend` 枚举 + `make_backend()` 工厂函数实现，
//! 避免 `Box<dyn>` 的虚表开销。

pub mod gdb;
pub mod mappings;
pub mod openocd;
pub mod probe_rs;
pub mod pyocd;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gdb::GdbBackend;
use openocd::OpenOcdBackend;
use probe_rs::ProbeRsBackend;
use pyocd::PyOcdBackend;
use serde::{Deserialize, Serialize};

// ============================================================================
// Backend trait
// ============================================================================

/// 烧录后端统一接口
///
/// 每个后端实现此 trait 提供：名称、二进制路径、参数构建、错误检测。
/// 通过 `FlashBackend` 枚举 + `make_backend()` 工厂函数零成本分发。
pub trait Backend: Send + Sync {
    /// 后端显示名称（v0.3 调试面板使用）
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    /// 可执行文件名称
    fn binary(&self) -> &'static str;
    /// 构建命令行参数列表
    fn build_args(&self, config: &FlashConfig) -> Vec<String>;
    /// 解析实际可执行文件路径（pyOCD 需要 venv 路径处理）
    fn resolve_binary(&self, config: &FlashConfig) -> String;
}

// ============================================================================
// 后端枚举 + 工厂
// ============================================================================

/// 支持的烧录后端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlashBackend {
    /// arm-none-eabi-gdb（默认）
    Gdb,
    /// OpenOCD
    OpenOcd,
    /// probe-rs
    ProbeRs,
    /// pyOCD（Python 烧录工具，支持 CMSIS-Pack）
    PyOcd,
}

impl FlashBackend {
    /// 从字符串解析后端类型
    pub fn from_str(s: &str) -> Self {
        match s {
            "openocd" => Self::OpenOcd,
            "probe-rs" => Self::ProbeRs,
            "pyocd" => Self::PyOcd,
            _ => Self::Gdb,
        }
    }

    /// 后端对应的 YAML key（如 "probe-rs"）
    pub fn yaml_key(&self) -> &'static str {
        match self {
            Self::Gdb => "gdb",
            Self::OpenOcd => "openocd",
            Self::ProbeRs => "probe-rs",
            Self::PyOcd => "pyocd",
        }
    }

    /// 后端对应的可执行文件名
    #[allow(dead_code)]
    pub fn binary(&self) -> &'static str {
        match self {
            Self::Gdb => GdbBackend.binary(),
            Self::OpenOcd => OpenOcdBackend.binary(),
            Self::ProbeRs => ProbeRsBackend.binary(),
            Self::PyOcd => PyOcdBackend.binary(),
        }
    }
}

/// 零成本工厂函数：根据枚举返回对应的 trait 实现引用
fn make_backend(be: FlashBackend) -> &'static dyn Backend {
    match be {
        FlashBackend::Gdb => &GdbBackend,
        FlashBackend::OpenOcd => &OpenOcdBackend,
        FlashBackend::ProbeRs => &ProbeRsBackend,
        FlashBackend::PyOcd => &PyOcdBackend,
    }
}

// ============================================================================
// 配置 & 结果
// ============================================================================

/// 烧录参数
#[derive(Debug, Clone)]
pub struct FlashConfig {
    /// 烧录后端
    pub backend: FlashBackend,
    /// 调试接口 key（如 "swd"、"stlink"）
    pub interface: String,
    /// 目标芯片 key（如 "stm32f4"）
    pub target: String,
    /// ELF 固件文件路径
    pub elf_path: String,
    /// GDB 远程端口（仅 GDB 后端使用）
    pub gdb_port: String,
    /// pyOCD 可执行文件路径（留空则使用 PATH 中的 pyocd）
    pub pyocd_path: String,
    /// 超时时间（秒），默认 60。0 表示无超时
    pub timeout_secs: u64,
    /// YAML 板子配置中的额外配置路径（如 OpenOCD target config）
    #[allow(dead_code)]
    pub board_config: String,
    /// YAML 板子配置中的额外命令行参数
    #[allow(dead_code)]
    pub board_extra_args: Vec<String>,
    /// 板子 ID（YAML key），为空时使用旧版映射
    #[allow(dead_code)]
    pub board_id: String,
}

/// 烧录结果，可作为 JSON 输出供 IDE 集成使用
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FlashResult {
    /// 烧录是否成功
    pub success: bool,
    /// 结果描述消息
    pub message: String,
    /// 实际执行的命令
    pub command: String,
    /// 标准输出
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    /// 标准错误
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

// ============================================================================
// 烧录执行
// ============================================================================

/// 执行烧录
///
/// 根据 `config.backend` 分发到对应后端，构建命令行并执行。
/// 返回 `FlashResult` 包含成功/失败标志、命令、输出等信息。
pub fn do_flash(config: &FlashConfig) -> FlashResult {
    // 基础校验：ELF 文件必须存在
    if !Path::new(&config.elf_path).is_file() {
        return FlashResult {
            success: false,
            message: format!("ELF 文件不存在或不是文件: {}", config.elf_path),
            command: String::new(),
            stdout: None,
            stderr: None,
        };
    }

    let be = make_backend(config.backend);
    let args = be.build_args(config);
    let binary = be.resolve_binary(config);
    let cmd_str = format!("{} {}", binary, args.join(" "));

    // 启动子进程
    let mut child = match Command::new(&binary)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return FlashResult {
                success: false,
                message: format!("无法启动 {}: {}", binary, e),
                command: cmd_str,
                stdout: None,
                stderr: None,
            };
        }
    };

    // 等待完成（带超时）
    let timeout = if config.timeout_secs > 0 {
        Duration::from_secs(config.timeout_secs)
    } else {
        Duration::from_secs(3600)
    };

    // 保存 PID 以便超时时杀进程
    let child_pid = child.id();

    // 先取出管道（必须在 spawn 之前，否则 child 被移动）
    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status);
    });

    let status_result = rx.recv_timeout(timeout);

    match status_result {
        Ok(Ok(status)) => {
            // 进程正常退出，读取 stdout/stderr
            let mut stdout_s = String::new();
            let mut stderr_s = String::new();
            if let Some(ref mut p) = child_stdout {
                use std::io::Read;
                let _ = p.read_to_string(&mut stdout_s);
            }
            if let Some(ref mut p) = child_stderr {
                use std::io::Read;
                let _ = p.read_to_string(&mut stderr_s);
            }

            let stdout = (!stdout_s.trim().is_empty()).then_some(stdout_s);
            let stderr = (!stderr_s.trim().is_empty()).then_some(stderr_s);

            // 判定成败
            let mut success = status.success();
            let mut fail_reason: Option<String> = None;

            if success {
                if let Some(reason) = detect_fatal_error(
                    stdout.as_deref().unwrap_or(""),
                    stderr.as_deref().unwrap_or(""),
                ) {
                    success = false;
                    fail_reason = Some(reason);
                }
            }

            let message = if success {
                format!(
                    "✅ 烧录成功！{} → {} 已复位运行。",
                    config.interface, config.target
                )
            } else if let Some(reason) = fail_reason {
                format!(
                    "❌ 烧录失败：{} (exit {:?}) — {} → {}",
                    reason,
                    status.code(),
                    config.interface,
                    config.target
                )
            } else {
                format!(
                    "❌ 烧录失败 (exit {:?}): {} → {}",
                    status.code(),
                    config.interface,
                    config.target
                )
            };

            FlashResult {
                success,
                message,
                command: cmd_str,
                stdout,
                stderr,
            }
        }
        Ok(Err(e)) => FlashResult {
            success: false,
            message: format!("❌ 进程异常退出: {}", e),
            command: cmd_str,
            stdout: None,
            stderr: None,
        },
        Err(_timeout) => {
            // 超时 — 通过 PID 杀掉进程
            let _ = Command::new("kill").arg("-9").arg(child_pid.to_string()).status();
            FlashResult {
                success: false,
                message: format!(
                    "❌ 烧录超时（{}s）！{} 可能不支持此芯片或探针未响应",
                    config.timeout_secs,
                    be.name(),
                ),
                command: cmd_str,
                stdout: None,
                stderr: None,
            }
        },
    }
}

// ============================================================================
// 错误检测
// ============================================================================

/// 扫描输出中的已知致命错误特征，返回可读的中文原因
fn detect_fatal_error(stdout: &str, stderr: &str) -> Option<String> {
    let hay = format!("{}\n{}", stdout, stderr);
    for &(needle, reason) in mappings::FATAL_ERROR_PATTERNS {
        if hay.contains(needle) {
            return Some(reason.to_string());
        }
    }
    None
}

// ============================================================================
// 工具路径解析
// ============================================================================

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_str() {
        assert_eq!(FlashBackend::from_str("gdb"), FlashBackend::Gdb);
        assert_eq!(FlashBackend::from_str("openocd"), FlashBackend::OpenOcd);
        assert_eq!(FlashBackend::from_str("probe-rs"), FlashBackend::ProbeRs);
        assert_eq!(FlashBackend::from_str("pyocd"), FlashBackend::PyOcd);
        assert_eq!(FlashBackend::from_str("unknown"), FlashBackend::Gdb);
    }

    #[test]
    fn detect_fatal_errors() {
        assert!(detect_fatal_error("", "could not connect: timeout")
            .unwrap()
            .contains("无法连接"));
        assert!(detect_fatal_error("No probes found.", "")
            .unwrap()
            .contains("探针"));
        assert!(detect_fatal_error("", "Error: open failed").is_some());
        assert!(detect_fatal_error("Downloading...\nErasing...", "").is_none());
    }
}
