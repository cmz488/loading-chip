//! 芯片自动检测 — 通过 probe-rs 识别连接的调试探针和芯片
//!
//! 检测接口始终可用；probe-rs 实现在 debug feature 控制下编译。

/// 检测到的芯片信息
#[derive(Debug, Clone)]
pub struct DetectedChip {
    /// 探针名称（如 "STLink V2"）
    pub probe_name: String,
    /// USB vendor ID
    pub vendor_id: u16,
    /// USB product ID
    pub product_id: u16,
    /// 探针序列号
    pub serial: Option<String>,
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
        .filter_map(|probe| {
            let probe_name = probe.identifier.clone();
            let vendor_id = probe.vendor_id;
            let product_id = probe.product_id;
            let serial = probe.serial_number.clone();

            // 推导接口类型
            let suggested_interface = match probe_name.to_lowercase() {
                s if s.contains("stlink") => "stlink".to_string(),
                s if s.contains("jlink") || s.contains("j-link") => "jlink".to_string(),
                s if s.contains("cmsis-dap") => "cmsis-dap".to_string(),
                s if s.contains("daplink") => "daplink".to_string(),
                s if s.contains("esp") => "usb-jtag".to_string(),
                s if s.contains("jtag") => "jtag".to_string(),
                _ => "swd".to_string(),
            };

            // 尝试打开探针并检测芯片
            let chip_name = match probe.open() {
                Ok(probe) => {
                    match probe.attach((), probe_rs::Permissions::default()) {
                        Ok(session) => {
                            let name = session.target().name.clone();
                            drop(session);
                            name
                        }
                        Err(_) => String::new(),
                    }
                }
                Err(_) => String::new(),
            };

            if chip_name.is_empty() {
                return None;
            }

            Some(DetectedChip {
                probe_name,
                vendor_id,
                product_id,
                serial,
                chip_name,
                board_id: None, // caller fills via BoardRegistry
                suggested_interface,
            })
        })
        .collect()
}
