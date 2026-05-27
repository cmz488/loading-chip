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
// OpenOCD RTT 客户端（telnet）
// ============================================================================

/// OpenOCD RTT 客户端 — 通过 telnet 连接 OpenOCD 并发送 RTT 命令
pub struct OpenOcdRtt {
    stream: Option<TcpStream>,
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl OpenOcdRtt {
    pub fn spawn(config: &RttConfig, sender: Sender<RttOutput>) -> std::io::Result<Self> {
        let addr = format!("127.0.0.1:{}", config.telnet_port);
        let mut stream = TcpStream::connect_timeout(
            &addr.parse().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
            })?,
            Duration::from_secs(5),
        )?;

        stream.set_read_timeout(Some(Duration::from_secs(1)))?;

        // 发送 RTT 初始化命令
        writeln!(stream, "rtt setup")?;
        writeln!(stream, "rtt start")?;
        writeln!(stream, "rtt server start 9090 0")?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        // 使用 BufReader 需要可变引用，这里用 clone
        let mut reader_stream = stream.try_clone()?;
        let handle = thread::spawn(move || {
            let mut reader = BufReader::new(&mut reader_stream);
            let mut line = String::new();
            loop {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let text = line.trim().to_string();
                        if !text.is_empty() {
                            let _ = sender.send(RttOutput {
                                channel: 0,
                                text,
                            });
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

        Ok(Self {
            stream: Some(stream),
            thread: Some(handle),
            running,
        })
    }
}

impl RttClient for OpenOcdRtt {
    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(ref mut stream) = self.stream {
            let _ = writeln!(stream, "rtt stop");
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for OpenOcdRtt {
    fn drop(&mut self) {
        self.stop();
    }
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
