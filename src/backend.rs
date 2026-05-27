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

/// 查找 probe-rs 可执行文件
/// 优先从 PATH 中查找，再尝试 ~/.cargo/bin/
fn resolve_probe_rs() -> String {
    // 检查 PATH
    for dir in std::env::var_os("PATH")
        .unwrap_or_default()
        .to_str()
        .unwrap_or("")
        .split(':') {
        let candidate = format!("{}/probe-rs", dir);
        if std::path::Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    // fallback: ~/.cargo/bin/probe-rs
    if let Some(home) = std::env::var_os("HOME") {
        let fallback = format!("{}/.cargo/bin/probe-rs", home.to_string_lossy());
        if std::path::Path::new(&fallback).is_file() {
            return fallback;
        }
    }
    "probe-rs".to_string() // 最后的尝试，让系统报错
}

/// 查找 pyOCD 可执行文件（现有逻辑在 PyOcdBackend::resolve_binary 中）
/// 此处供 GDB Server 模式使用
#[allow(dead_code)]
fn resolve_pyocd(pyocd_path: &str) -> String {
    if !pyocd_path.is_empty() {
        return pyocd_path.to_string();
    }
    if let Ok(p) = std::env::var("PYOCD_PATH") {
        if std::path::Path::new(&p).is_file() {
            return p;
        }
    }
    "pyocd".to_string()
}

// ============================================================================
// GDB Server 管理 — 调试模式自动启动
// ============================================================================

/// GDB Server 进程管理
///
/// 调试模式下根据后端类型自动启动对应的 GDB Server，并在退出时自动清理。
/// - probe-rs: `probe-rs gdb --chip <CHIP>`
/// - openocd:  `openocd -f <config> -c "gdb_port <PORT>"`
/// - pyocd:    `pyocd gdbserver -t <TARGET> -p <PORT>`
/// - gdb:      不启动 GDB Server（假设用户已手动启动）
#[derive(Debug)]
pub struct GdbServerProcess {
    child: Option<std::process::Child>,
    pub port: u16,
}

impl Drop for GdbServerProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

impl GdbServerProcess {
    /// 启动 GDB Server
    ///
    /// 返回 `None` 表示该后端不需要启动 GDB Server（如 gdb 后端）。
    pub fn spawn(
        backend: &str,
        target: &str,
        interface: Option<&str>,
        port: u16,
    ) -> Option<std::io::Result<Self>> {
        match backend {
            "probe-rs" => Some(Self::spawn_probe_rs(target, port)),
            "openocd" => Some(Self::spawn_openocd(target, interface, port)),
            "pyocd" => Some(Self::spawn_pyocd(target, port)),
            _ => None, // 裸 GDB 模式：不启动 Server
        }
    }

    /// probe-rs: `probe-rs gdb --chip <CHIP>`
    fn spawn_probe_rs(target: &str, port: u16) -> std::io::Result<Self> {
        let chip = mappings::probe_rs_chip(target);
        let conn_str = format!("localhost:{}", port);
        let probe_rs_bin = resolve_probe_rs();

        let child = std::process::Command::new(&probe_rs_bin)
            .args(["gdb", "--gdb-connection-string", &conn_str, "--chip", chip])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        Ok(Self {
            child: Some(child),
            port,
        })
    }

    /// OpenOCD: `openocd -f <interface> -f <target> -c "gdb_port <PORT>" -c "bindto 0.0.0.0"`
    fn spawn_openocd(
        target: &str,
        interface: Option<&str>,
        port: u16,
    ) -> std::io::Result<Self> {
        let mut args: Vec<String> = Vec::new();

        let is_esp = target.starts_with("esp");
        if is_esp {
            args.push("-f".into());
            args.push(mappings::openocd_target_cfg(target).into());
        } else if let Some(iface) = interface {
            args.push("-f".into());
            args.push(mappings::openocd_interface_cfg(iface).into());
            args.push("-f".into());
            args.push(mappings::openocd_target_cfg(target).into());
            if iface == "swd" {
                args.push("-c".into());
                args.push("transport select swd".into());
            }
        } else {
            // 没有 interface 时假设默认
            args.push("-f".into());
            args.push(mappings::openocd_interface_cfg("swd").into());
            args.push("-f".into());
            args.push(mappings::openocd_target_cfg(target).into());
        }

        // GDB 服务器模式：设置端口，不自动烧录
        args.push("-c".into());
        args.push(format!("gdb_port {}", port));
        args.push("-c".into());
        args.push("bindto 0.0.0.0".into());

        let child = std::process::Command::new("openocd")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        Ok(Self {
            child: Some(child),
            port,
        })
    }

    /// pyOCD: `pyocd gdbserver -t <TARGET> -p <PORT>`
    fn spawn_pyocd(target: &str, port: u16) -> std::io::Result<Self> {
        let pyocd_target = mappings::pyocd_target(target);

        let child = std::process::Command::new("pyocd")
            .args(["gdbserver", "-t", pyocd_target, "-p", &port.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        Ok(Self {
            child: Some(child),
            port,
        })
    }

    /// 等待 GDB Server 端口就绪（最多等 10 秒）
    /// 同时尝试 IPv4 (127.0.0.1) 和 IPv6 ([::1])，因为工具可能只绑其中一个
    pub fn wait_ready(&mut self) -> std::io::Result<()> {
        use std::net::{TcpStream, ToSocketAddrs};
        use std::time::Instant;

        let deadline = Instant::now() + Duration::from_secs(10);

        // 要尝试的所有 localhost 地址
        let addrs: Vec<std::net::SocketAddr> = [
            format!("127.0.0.1:{}", self.port),
            format!("[::1]:{}", self.port),
        ]
        .iter()
        .filter_map(|s| s.to_socket_addrs().ok()?.next())
        .collect();

        // 检查子进程是否还活着
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("GDB Server 已提前退出 (exit: {:?})", status.code()),
                    ));
                }
                Ok(None) => {} // 子进程还在跑
                Err(e) => {
                    return Err(std::io::Error::other(
                        format!("无法检查子进程状态: {}", e),
                    ));
                }
            }
        }

        // 轮询 TCP 端口（任一地址成功即视为就绪）
        while Instant::now() < deadline {
            for addr in &addrs {
                match TcpStream::connect_timeout(addr, Duration::from_millis(200)) {
                    Ok(_) => return Ok(()),
                    Err(_) => continue,
                }
            }

            // 检查子进程是否挂了
            if let Some(ref mut child) = self.child {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        format!("GDB Server 已退出 (exit: {:?})", status.code()),
                    ));
                }
            }

            std::thread::sleep(Duration::from_millis(300));
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("GDB Server {} 秒未在端口 {} 就绪", 10, self.port),
        ))
    }

    /// 终止 GDB Server
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

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
