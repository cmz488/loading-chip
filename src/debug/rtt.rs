//! RTT (Real-Time Transfer) 客户端
//!
//! 通过调试探针实现双向实时数据传输，无需占用 UART 串口。
//! 支持多种后端：probe-rs（库 API 直接读取 RTT）、OpenOCD（telnet）、pyOCD（telnet）。
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
    pub channel: u8,
    pub text: String,
}

/// RTT 客户端 trait
pub trait RttClient: Send {
    fn is_running(&self) -> bool;
    fn stop(&mut self);
}

// ============================================================================
// RTT 配置
// ============================================================================

#[derive(Debug, Clone)]
pub struct RttConfig {
    pub backend: RttBackend,
    pub chip: String,
    pub probe: String,
    pub telnet_port: u16,
    pub elf_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RttBackend {
    ProbeRs,
    OpenOcd,
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
// probe-rs RTT 客户端（库 API，直接读取 RAM RTT 缓冲区）
// ============================================================================

pub struct ProbeRsRtt {
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl ProbeRsRtt {
    pub fn spawn(config: &RttConfig, sender: Sender<RttOutput>) -> std::io::Result<Self> {
        #[cfg(not(feature = "debug"))]
        {
            let _ = sender.send(RttOutput { channel: 1, text: "RTT 需要 debug feature".into() });
            return Ok(Self { thread: None, running: Arc::new(AtomicBool::new(false)) });
        }

        #[cfg(feature = "debug")]
        {
            let running = Arc::new(AtomicBool::new(true));
            let running_clone = Arc::clone(&running);
            let chip = config.chip.clone();
            let probe_desc = config.probe.clone();
            let elf_path = config.elf_path.clone();

            let handle = thread::Builder::new()
                .name("probe-rs-rtt".into())
                .spawn(move || {
                    if let Err(e) = probe_rs_rtt_loop(&chip, &probe_desc, &running_clone, &sender, elf_path.as_deref()) {
                        let _ = sender.send(RttOutput { channel: 1, text: format!("RTT 错误: {}", e) });
                    }
                })
                .map_err(std::io::Error::other)?;

            Ok(Self { thread: Some(handle), running })
        }
    }
}

impl RttClient for ProbeRsRtt {
    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
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
// probe-rs 库 API RTT 主循环
// ============================================================================

#[cfg(feature = "debug")]
fn probe_rs_rtt_loop(
    chip: &str,
    probe_desc: &str,
    running: &AtomicBool,
    sender: &Sender<RttOutput>,
    elf_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use probe_rs::probe::list::Lister;
    use probe_rs::rtt::{Rtt, ScanRegion};
    use probe_rs::Permissions;

    let _ = sender.send(RttOutput { channel: 0, text: "🔍 正在连接探针...".into() });

    let lister = Lister::new();
    let probes = lister.list_all();
    if probes.is_empty() {
        let _ = sender.send(RttOutput { channel: 1, text: "❌ 未发现调试探针".into() });
        return Ok(());
    }

    let probe_idx = if probe_desc.is_empty() {
        0
    } else {
        probes.iter().position(|p| {
            format!("{:04x}:{:04x}:{}", p.vendor_id, p.product_id,
                p.serial_number.as_deref().unwrap_or(""))
                .contains(probe_desc)
        }).unwrap_or(0)
    };

    let _ = sender.send(RttOutput {
        channel: 0,
        text: format!("🔌 探针: {:04x}:{:04x}",
            probes[probe_idx].vendor_id, probes[probe_idx].product_id),
    });

    let probe = probes[probe_idx].open()?;
    let mut session = probe.attach(chip, Permissions::default())?;
    let _ = sender.send(RttOutput { channel: 0, text: format!("✅ 已连接 {}", chip) });

    let mut core = session.core(0)?;

    // 1. 优先 ELF 符号查找
    let mut rtt: Option<Rtt> = None;
    if let Some(ep) = elf_path {
        if let Some(addr) = find_rtt_symbol_in_elf(ep) {
            let _ = sender.send(RttOutput {
                channel: 0,
                text: format!("📍 _SEGGER_RTT @ 0x{:08X}", addr),
            });
            match Rtt::attach_region(&mut core, &ScanRegion::Exact(addr)) {
                Ok(r) => { rtt = Some(r); }
                Err(_) => {
                    let _ = sender.send(RttOutput {
                        channel: 1,
                        text: "⚠️ ELF 符号地址无效，回退到内存扫描...".into(),
                    });
                }
            }
        }
    }

    // 2. 回退：已知地址扫描
    if rtt.is_none() {
        let _ = sender.send(RttOutput { channel: 0, text: "🔎 扫描 RTT 控制块...".into() });
        rtt = attach_rtt_fallback(&mut core, chip, running);
    }

    // 3. 最终回退：全量 RAM 扫描
    let mut rtt = match rtt {
        Some(r) => r,
        None => {
            let _ = sender.send(RttOutput { channel: 0, text: "🔎 全量 RAM 扫描...".into() });
            Rtt::attach(&mut core).map_err(|e| format!("RTT attach 失败: {}", e))?
        }
    };

    let _ = sender.send(RttOutput {
        channel: 0,
        text: format!("✅ RTT 就绪 ({} 通道)", rtt.up_channels().len()),
    });

    let mut buf = vec![0u8; 4096];
    while running.load(Ordering::SeqCst) {
        for i in 0..rtt.up_channels().len() {
            if let Some(ch) = rtt.up_channel(i) {
                match ch.read(&mut core, &mut buf) {
                    Ok(count) if count > 0 => {
                        let text = String::from_utf8_lossy(&buf[..count]).to_string();
                        if sender.send(RttOutput { channel: i as u8, text }).is_err() {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}

#[cfg(feature = "debug")]
fn attach_rtt_fallback(
    core: &mut probe_rs::Core,
    chip: &str,
    running: &AtomicBool,
) -> Option<probe_rs::rtt::Rtt> {
    use probe_rs::rtt::{Rtt, ScanRegion};

    // ESP32 已知 RTT 地址
    if chip.to_lowercase().contains("esp32") {
        let known: &[u64] = &[0x3FC9_1200, 0x3FC9_1000, 0x3FC8_8000, 0x3FFB_0000];
        for &addr in known {
            if !running.load(Ordering::SeqCst) { break; }
            for offset in [-0x2000i64, -0x1000, 0, 0x1000, 0x2000] {
                let probe = (addr as i64 + offset) as u64;
                if !(0x3FC8_0000..=0x3FD0_0000).contains(&probe) { continue; }
                if let Ok(r) = Rtt::attach_region(core, &ScanRegion::Exact(probe)) {
                    return Some(r);
                }
            }
        }
        // ESP32 SRAM1 范围扫描
        if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(0x3FC8_8000..0x3FD0_0000)) {
            return Some(r);
        }
    }

    None
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
