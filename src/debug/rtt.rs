//! RTT (Real-Time Transfer) 客户端
//!
//! 通过调试探针实现双向实时数据传输，无需占用 UART 串口。
//! 支持多种后端：probe-rs（CLI 命令）、OpenOCD（telnet）。
//!
//! 设计：后台线程持续读取 RTT 输出，通过 channel 发送到 TUI。

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;

/// RTT 输出记录
#[derive(Debug, Clone)]
pub struct RttOutput {
    /// 通道号（RTT 支持多通道，通常 channel 0 = 终端输出）
    pub channel: u8,
    /// 文本内容
    pub text: String,
}

/// RTT 客户端 trait
pub trait RttClient: Send {
    /// 是否仍在运行
    fn is_running(&self) -> bool;

    /// 停止 RTT 采集
    fn stop(&mut self);
}

// ============================================================================
// RTT 配置
// ============================================================================

/// RTT 连接配置
#[derive(Debug, Clone)]
pub struct RttConfig {
    /// 后端类型：probe-rs / openocd / none
    pub backend: RttBackend,
    /// 目标芯片名（probe-rs 需要）
    pub chip: String,
    /// 调试探针 VID:PID（probe-rs 需要）
    pub probe: String,
    /// OpenOCD telnet 端口
    pub telnet_port: u16,
    /// ELF 文件路径，用于从符号表查找 _SEGGER_RTT 地址
    pub elf_path: Option<String>,
}

/// RTT 后端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RttBackend {
    /// probe-rs CLI: probe-rs rtt --chip <chip>
    ProbeRs,
    /// OpenOCD telnet: rtt setup / rtt start / rtt server
    OpenOcd,
    /// 不使用 RTT
    None,
}

impl RttBackend {
    pub fn from_str(s: &str) -> Self {
        match s {
            "probe-rs" | "probers" => Self::ProbeRs,
            "openocd" => Self::OpenOcd,
            _ => Self::None,
        }
    }
}

// ============================================================================
// probe-rs RTT 客户端
// ============================================================================

/// probe-rs RTT 客户端 — 启动 probe-rs rtt 子进程
pub struct ProbeRsRtt {
    child: Option<Child>,
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl ProbeRsRtt {
    pub fn spawn(config: &RttConfig, sender: Sender<RttOutput>) -> std::io::Result<Self> {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        // 尝试 ELF 符号查找 — 优先定位 _SEGGER_RTT 地址
        if let Some(ref elf_path) = config.elf_path {
            if let Some(addr) = find_rtt_symbol_in_elf(elf_path) {
                let _ = sender.send(RttOutput {
                    channel: 0,
                    text: format!("📍 在 ELF 中找到 _SEGGER_RTT @ 0x{:08X}", addr),
                });
            } else {
                let _ = sender.send(RttOutput {
                    channel: 1,
                    text: "⚠️ ELF 中未找到 _SEGGER_RTT 符号，将使用内存扫描...".into(),
                });
            }
        }

        let mut cmd = Command::new("probe-rs");
        cmd.arg("rtt");
        cmd.arg("--chip");
        cmd.arg(&config.chip);

        if !config.probe.is_empty() {
            cmd.arg("--probe");
            cmd.arg(&config.probe);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        // 后台线程读取 stdout
        let running_stdout = Arc::clone(&running_clone);
        let sender_stdout = sender.clone();
        let handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if !running_stdout.load(Ordering::SeqCst) {
                    break;
                }
                let _ = sender_stdout.send(RttOutput {
                    channel: 0,
                    text: line,
                });
            }
        });

        // stderr → channel 1
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }
                let _ = sender.send(RttOutput {
                    channel: 1,
                    text: line,
                });
            }
        });

        Ok(Self {
            child: Some(child),
            thread: Some(handle),
            running,
        })
    }
}

impl RttClient for ProbeRsRtt {
    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProbeRsRtt {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Telnet RTT 客户端（OpenOCD / pyOCD 共用）
// ============================================================================

/// Telnet-based RTT client.
///
/// `startup_fn` is called once after connecting to issue backend-specific
/// RTT setup/tracking commands.
pub struct TelnetRtt {
    stream: Option<TcpStream>,
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl TelnetRtt {
    pub fn spawn(
        telnet_addr: &str,
        startup_fn: impl FnOnce(&mut TcpStream) -> std::io::Result<()> + Send + 'static,
        sender: Sender<RttOutput>,
    ) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect_timeout(
            &telnet_addr.parse().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
            Duration::from_secs(10),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        startup_fn(&mut stream)?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let mut reader_stream = stream.try_clone()?;

        let handle = thread::spawn(move || {
            let mut reader = BufReader::new(&mut reader_stream);
            let mut line = String::new();
            loop {
                if !running_clone.load(Ordering::SeqCst) { break; }
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            let _ = sender.send(RttOutput { channel: 0, text: trimmed });
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self { stream: Some(stream), thread: Some(handle), running })
    }
}

impl RttClient for TelnetRtt {
    fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }
    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
        if let Some(stream) = self.stream.take() { let _ = stream.shutdown(std::net::Shutdown::Both); }
    }
}

impl Drop for TelnetRtt {
    fn drop(&mut self) { self.stop(); }
}

// ============================================================================
// OpenOCD RTT（telnet）
// ============================================================================

/// 启动 OpenOCD telnet-based RTT
pub fn spawn_openocd_rtt(telnet_port: u16, sender: Sender<RttOutput>) -> std::io::Result<TelnetRtt> {
    let addr = format!("127.0.0.1:{}", telnet_port);
    TelnetRtt::spawn(&addr, |stream| {
        writeln!(stream, "rtt setup")?;
        writeln!(stream, "rtt start")?;
        writeln!(stream, "rtt server start 9090 0")?;
        Ok(())
    }, sender)
}

// ============================================================================
// pyOCD RTT（telnet）
// ============================================================================

/// 启动 pyOCD telnet-based RTT
pub fn spawn_pyocd_rtt(telnet_port: u16, sender: Sender<RttOutput>) -> std::io::Result<TelnetRtt> {
    let addr = format!("127.0.0.1:{}", telnet_port);
    TelnetRtt::spawn(&addr, |stream| {
        writeln!(stream, "rtt")?;
        Ok(())
    }, sender)
}

// ============================================================================
// RTT 通道（统一接收端）
// ============================================================================

/// RTT 接收通道，供 TUI 非阻塞读取
pub struct RttChannel {
    rx: crossbeam_channel::Receiver<RttOutput>,
}

impl RttChannel {
    pub fn new(rx: crossbeam_channel::Receiver<RttOutput>) -> Self {
        Self { rx }
    }

    /// 非阻塞读取所有 RTT 输出
    pub fn drain(&self) -> Vec<RttOutput> {
        let mut outputs = Vec::new();
        while let Ok(out) = self.rx.try_recv() {
            outputs.push(out);
        }
        outputs
    }
}

// ============================================================================
// ELF 符号查找
// ============================================================================

/// 从 ELF 文件中查找 _SEGGER_RTT 符号地址
///
/// 使用 `object` crate 解析 ELF 符号表。
#[cfg(feature = "debug")]
fn find_rtt_symbol_in_elf(elf_path: &str) -> Option<u64> {
    use object::{Object, ObjectSymbol};

    let data = std::fs::read(elf_path).ok()?;
    let obj = object::File::parse(&*data).ok()?;
    for sym in obj.symbols() {
        if let Ok(name) = sym.name() {
            if name == "_SEGGER_RTT" {
                return Some(sym.address());
            }
        }
    }
    None
}

/// 无 debug feature 时的存根
#[cfg(not(feature = "debug"))]
fn find_rtt_symbol_in_elf(_elf_path: &str) -> Option<u64> {
    None
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtt_backend_from_str() {
        assert_eq!(RttBackend::from_str("probe-rs"), RttBackend::ProbeRs);
        assert_eq!(RttBackend::from_str("probers"), RttBackend::ProbeRs);
        assert_eq!(RttBackend::from_str("openocd"), RttBackend::OpenOcd);
        assert_eq!(RttBackend::from_str("none"), RttBackend::None);
        assert_eq!(RttBackend::from_str("unknown"), RttBackend::None);
    }

    #[test]
    fn rtt_output_clone() {
        let out = RttOutput {
            channel: 0,
            text: "hello".into(),
        };
        assert_eq!(out.channel, 0);
        assert_eq!(out.text, "hello");
    }

    #[test]
    fn rtt_channel_drain_empty() {
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(tx);
        let ch = RttChannel::new(rx);
        assert!(ch.drain().is_empty());
    }

    #[test]
    fn rtt_channel_drain_with_data() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(RttOutput {
            channel: 0,
            text: "data".into(),
        })
        .unwrap();
        let ch = RttChannel::new(rx);
        let drained = ch.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].text, "data");
    }
}
