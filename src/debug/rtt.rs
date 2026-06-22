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

#[derive(Clone)]
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
            return Err(std::io::Error::other("RTT 功能需要启用 debug feature（默认已启用）"));
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

    // probe.attach() 会 halt 核心 — 需要 reset 让固件跑起来，
    // 否则 .data 段（包含 RTT 控制块）永远不会从 Flash 解压到 RAM
    let _ = sender.send(RttOutput { channel: 0, text: "🔄 复位芯片...".into() });
    // reset_and_halt: 复位后短暂 halt（让 startup code 执行 .data 初始化）
    // 如果芯片没有在 timeout 内 halt（正常情况：自由运行），忽略错误
    let _ = core.reset_and_halt(Duration::from_millis(200));
    // 确保芯片在运行状态
    let _ = core.run();
    let _ = sender.send(RttOutput { channel: 0, text: "⏳ 等待 RTT 就绪...".into() });
    thread::sleep(Duration::from_millis(500));

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
                        text: "⚠️ ELF 符号地址无效，回退到 SRAM 扫描...".into(),
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

    // 3. 最终回退：多区域 SRAM 扫描
    let mut rtt = match rtt {
        Some(r) => r,
        None => {
            // 按可能性排序扫描常见 SRAM 区域
            let regions: &[std::ops::Range<u64>] = &[
                0x2000_0000..0x2004_0000, // ARM Cortex-M 主 SRAM (256KB)
                0x2400_0000..0x2408_0000, // STM32H7 AXI SRAM
                0x3000_0000..0x3004_0000, // STM32H7/F7 SDRAM bank
            ];
            let mut found = None;
            for region in regions {
                if !running.load(Ordering::SeqCst) { break; }
                let _ = sender.send(RttOutput {
                    channel: 0,
                    text: format!("🔎 扫描 SRAM 0x{:08X}-0x{:08X}...", region.start, region.end),
                });
                if let Ok(r) = Rtt::attach_region(&mut core, &ScanRegion::range(region.clone())) {
                    found = Some(r);
                    break;
                }
            }
            match found {
                Some(r) => r,
                None => {
                    let _ = sender.send(RttOutput { channel: 0, text: "🔎 默认 RAM 扫描...".into() });
                    Rtt::attach(&mut core).map_err(|e| format!("RTT attach 失败: {}", e))?
                }
            }
        }
    };

    let _ = sender.send(RttOutput {
        channel: 0,
        text: format!("✅ RTT 就绪 ({} 通道)", rtt.up_channels().len()),
    });

    // 每个通道的行缓冲：累积不完整行数据，按 '\n' 拆分为完整行输出
    let num_channels = rtt.up_channels().len();
    let mut line_bufs: Vec<String> = vec![String::new(); num_channels];
    let mut buf = vec![0u8; 4096];

    while running.load(Ordering::SeqCst) {
        for i in 0..num_channels {
            if let Some(ch) = rtt.up_channel(i) {
                match ch.read(&mut core, &mut buf) {
                    Ok(count) if count > 0 => {
                        line_bufs[i].push_str(&String::from_utf8_lossy(&buf[..count]));
                        while let Some(nl) = line_bufs[i].find('\n') {
                            let line = line_bufs[i][..nl].trim_end_matches('\r').to_string();
                            line_bufs[i].drain(..=nl);
                            if !line.is_empty() {
                                let out = RttOutput { channel: i as u8, text: line };
                                if sender.send(out).is_err() { return Ok(()); }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // 退出前 flush 各通道缓冲中的残余数据
    for (i, buf) in line_bufs.iter().enumerate() {
        if !buf.is_empty() {
            let _ = sender.send(RttOutput { channel: i as u8, text: buf.clone() });
        }
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

    let chip_lower = chip.to_lowercase();

    // === ESP32 系列（XTensa / RISC-V）===
    if chip_lower.contains("esp") {
        // ESP32-S3/C3: SRAM @ 0x3FC8_0000-0x3FD0_0000
        // ESP32:      SRAM @ 0x3FFB_0000 (DRAM)
        let known: &[u64] = &[0x3FC9_1200, 0x3FC9_1000, 0x3FC8_8000, 0x3FFB_0000, 0x3FCA_0000];
        for &addr in known {
            if !running.load(Ordering::SeqCst) { break; }
            for offset in [-0x2000i64, -0x1000, 0, 0x1000, 0x2000] {
                let probe = (addr as i64 + offset) as u64;
                if probe < 0x3FC8_0000 || probe > 0x3FD0_0000 { continue; }
                if let Ok(r) = Rtt::attach_region(core, &ScanRegion::Exact(probe)) {
                    return Some(r);
                }
            }
        }
        // ESP32 DRAM + SRAM1 范围扫描
        for range in [0x3FC8_8000..0x3FD0_0000, 0x3FFB_0000..0x3FFE_0000] {
            if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(range)) {
                return Some(r);
            }
        }
        return None;
    }

    // === ARM Cortex-M 通用 ===
    // 所有 Cortex-M MCU 的主 SRAM 起始于 0x20000000
    // 部分高端芯片有额外的 SRAM 区域

    // Region 1: 主 SRAM @ 0x20000000 (覆盖 MSPM0/STM32F1-F4-G0/nRF52/RP2040/GD32/AT32)
    if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(0x2000_0000..0x2004_0000)) {
        return Some(r);
    }

    // Region 2: STM32H7 AXI SRAM @ 0x24000000 (STM32H743/H750)
    if chip_lower.contains("stm32h7") || chip_lower.contains("h7") {
        if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(0x2400_0000..0x2408_0000)) {
            return Some(r);
        }
    }

    // Region 3: STM32H7 DTCM @ 0x20000000 (already covered above, but ITCM is @ 0x00000000)
    // Region 4: Some STM32F7/H7 have SRAM @ 0x30000000 (SDRAM bank)
    if chip_lower.contains("stm32h7") || chip_lower.contains("stm32f7") {
        if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(0x3000_0000..0x3004_0000)) {
            return Some(r);
        }
    }

    // Region 5: Broad SRAM scan for unknown ARM chips (慢但全面)
    // 仅对不在已知列表中的芯片执行
    if !chip_lower.contains("stm32") && !chip_lower.contains("mspm0")
        && !chip_lower.contains("nrf") && !chip_lower.contains("rp2040")
    {
        // 扫描标准 ARM SRAM 区域前 256KB
        if let Ok(r) = Rtt::attach_region(core, &ScanRegion::range(0x2000_0000..0x2004_0000)) {
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
// RTT 客户端工厂
// ============================================================================

/// 根据后端类型创建 RTT 客户端和可选的子进程句柄
///
/// TUI 模式的 RTT 客户端工厂函数。
///
/// # Returns
/// - `Ok((client, child))` — RTT 客户端 + OpenOCD/pyOCD 子进程（probe-rs 时为 None）
/// - `Err(msg)` — 启动失败的原因
pub fn create_rtt_client(
    backend: &str,
    target: &str,
    interface: &str,
    elf_path: &str,
    gdb_port: u16,
    pyocd_path: &str,
    tx: Sender<RttOutput>,
) -> Result<(Box<dyn RttClient>, Option<std::process::Child>), String> {
    match backend {
        "openocd" => {
            let icfg = crate::backend::mappings::openocd_interface_cfg(interface);
            let tcfg = crate::backend::mappings::openocd_target_cfg(target);
            let child = std::process::Command::new("openocd")
                .args(["-f", icfg, "-f", tcfg, "-c", &format!("gdb_port {}", gdb_port)])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("OpenOCD 启动失败: {}", e))?;
            std::thread::sleep(std::time::Duration::from_millis(500));
            let client = spawn_openocd_rtt(4444, tx)
                .map_err(|e| format!("OpenOCD RTT 连接失败: {}", e))?;
            Ok((Box::new(client), Some(child)))
        }
        "pyocd" => {
            let t = crate::backend::mappings::pyocd_target(target);
            let bin = if pyocd_path.is_empty() {
                "pyocd".to_string()
            } else {
                pyocd_path.to_string()
            };
            let child = std::process::Command::new(&bin)
                .args([
                    "gdbserver", "--target", t,
                    "--port", &gdb_port.to_string(),
                    "--telnet-port", "4444",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("pyOCD 启动失败: {}", e))?;
            std::thread::sleep(std::time::Duration::from_millis(500));
            let client = spawn_pyocd_rtt(4444, tx)
                .map_err(|e| format!("pyOCD RTT 连接失败: {}", e))?;
            Ok((Box::new(client), Some(child)))
        }
        "gdb" => Err("GDB 模式下 RTT 不可用".into()),
        _ => {
            let cfg = RttConfig {
                backend: RttBackend::ProbeRs,
                chip: target.to_string(),
                probe: String::new(),
                telnet_port: gdb_port,
                elf_path: if elf_path.is_empty() { None } else { Some(elf_path.to_string()) },
            };
            let client = ProbeRsRtt::spawn(&cfg, tx)
                .map_err(|e| format!("probe-rs RTT 启动失败: {}", e))?;
            Ok((Box::new(client), None))
        }
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
