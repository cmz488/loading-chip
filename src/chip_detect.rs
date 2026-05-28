//! 芯片自动检测 — 通过 probe-rs 识别连接的调试探针和芯片
//!
//! 检测接口始终可用；probe-rs 实现在 debug feature 控制下编译。

use std::io;

/// 检测到的芯片信息
#[derive(Debug, Clone)]
pub struct DetectedChip {
    /// 探针名称（如 "STLink V2"）
    pub probe_name: String,
    /// probe-rs 返回的芯片名（如 "STM32F407VG"）
    pub chip_name: String,
    /// 对应的 boards.yaml board ID（如果找到映射）
    pub board_id: Option<String>,
    /// 推荐的接口类型（从探针类型推导）
    pub suggested_interface: String,
}

/// 检测所有已连接的调试探针和芯片
///
/// 不使用 probe-rs 时返回空列表。
pub fn detect_chips() -> Vec<DetectedChip> {
    #[cfg(not(feature = "debug"))]
    {
        Vec::new()
    }

    #[cfg(feature = "debug")]
    {
        detect_chips_impl()
    }
}

#[cfg(feature = "debug")]
fn detect_chips_impl() -> Vec<DetectedChip> {
    use probe_rs::probe::list::Lister;

    let lister = Lister::new();
    let probes = lister.list_all();

    probes
        .into_iter()
        .map(|probe| {
            let probe_name = probe.identifier.clone();
            let suggested_interface = match probe_name.to_lowercase() {
                s if s.contains("stlink") => "stlink".to_string(),
                s if s.contains("jlink") || s.contains("j-link") => "jlink".to_string(),
                s if s.contains("cmsis-dap") => "cmsis-dap".to_string(),
                s if s.contains("daplink") => "daplink".to_string(),
                s if s.contains("esp") => "usb-jtag".to_string(),
                s if s.contains("jtag") => "jtag".to_string(),
                _ => "swd".to_string(),
            };

            // 尝试自动检测芯片（可能失败：克隆探头 / 未知芯片 / 权限不足）
            let chip_name = match probe.open() {
                Ok(attached) => match attached.attach((), probe_rs::Permissions::default()) {
                    Ok(session) => {
                        let name = session.target().name.clone();
                        drop(session);
                        name
                    }
                    Err(_) => String::new(),
                },
                Err(_) => String::new(),
            };

            // 即使芯片检测失败也返回探头信息（TUI 显示探头名，用户手动选芯片）
            DetectedChip {
                probe_name,
                chip_name,
                board_id: None,
                suggested_interface,
            }
        })
        .collect()
}
/// 运行detect终端命令
pub fn run_detect() -> io::Result<()> {
    let boards = detect_chips();
    if boards.is_empty() {
        eprintln!("no board find");
        return Ok(());
    }
    let num = boards.len();
    eprintln!("已检测到 {} 块板子", num);
    // 计算各列最大宽度用于对齐
    let max_chip = boards.iter().map(|b| b.chip_name.len()).max().unwrap_or(0);
    let max_board = boards
        .iter()
        .map(|b| b.board_id.as_deref().unwrap_or("").len())
        .max()
        .unwrap_or(0);
    let max_iface = boards
        .iter()
        .map(|b| b.suggested_interface.len())
        .max()
        .unwrap_or(0);
    let max_probe = boards.iter().map(|b| b.probe_name.len()).max().unwrap_or(0);

    for board in boards {
        let chip = board.chip_name;
        let board_id = board.board_id.as_deref().unwrap_or("");
        println!(
            "[chip: \"{:>max_chip$}\"；board: \"{:>max_board$}\"；interface: \"{:>max_iface$}\"；probe_name: \"{:>max_probe$}\"]",
            chip,
            board_id,
            board.suggested_interface,
            board.probe_name,
            max_chip = max_chip,
            max_board = max_board,
            max_iface = max_iface,
            max_probe = max_probe,
        );
    }
    Ok(())
}
