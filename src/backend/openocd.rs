//! OpenOCD 烧录后端
//!
//! 直接调用 OpenOCD 完成固件烧录，无需外部 GDB Server。
//! 所有芯片统一使用 interface + target 分离模式（兼容主线 OpenOCD 和 Espressif 分支）。

use crate::backend::mappings;
use crate::backend::Backend;
use crate::FlashConfig;

/// OpenOCD 后端
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenOcdBackend;

impl Backend for OpenOcdBackend {
    fn name(&self) -> &'static str {
        "OpenOCD"
    }

    fn binary(&self) -> &'static str {
        "openocd"
    }

    fn build_args(&self, config: &FlashConfig) -> Vec<String> {
        let mut args = vec![
            "-f".to_string(),
            mappings::openocd_interface_cfg(&config.interface).into(),
            "-f".to_string(),
            mappings::openocd_target_cfg(&config.target).into(),
        ];

        // 自动选择传输协议 — 仅非 ESP 芯片需要（ESP target 配置自带传输选择）
        if !config.target.starts_with("esp") {
            let iface = &config.interface;
            if crate::backend::mappings::is_swd_probe(iface) {
                args.push("-c".into());
                args.push("transport select swd".into());
            } else if crate::backend::mappings::is_jtag_probe(iface) {
                args.push("-c".into());
                args.push("transport select jtag".into());
            }
        }

        args.push("-c".into());
        args.push(format!("program {} verify reset exit", config.elf_path));

        args
    }

    fn resolve_binary(&self, _config: &FlashConfig) -> String {
        self.binary().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FlashConfig {
        FlashConfig {
            backend: crate::FlashBackend::OpenOcd,
            interface: "swd".into(),
            target: "stm32f4".into(),
            elf_path: "a.elf".into(),
            gdb_port: String::new(),
            pyocd_path: String::new(),
            timeout_secs: 0,
            board_config: String::new(),
            board_extra_args: vec![],
            board_id: String::new(),
        }
    }

    fn esp_cfg() -> FlashConfig {
        FlashConfig {
            backend: crate::FlashBackend::OpenOcd,
            interface: "usb-jtag".into(),
            target: "esp32s3".into(),
            elf_path: "a.elf".into(),
            gdb_port: String::new(),
            pyocd_path: String::new(),
            timeout_secs: 0,
            board_config: String::new(),
            board_extra_args: vec![],
            board_id: String::new(),
        }
    }

    #[test]
    fn args_swd() {
        let args = OpenOcdBackend.build_args(&cfg());
        assert!(args.iter().any(|s| s == "interface/cmsis-dap.cfg"));
        assert!(args.iter().any(|s| s == "target/stm32f4x.cfg"));
        assert!(args.iter().any(|s| s == "transport select swd"));
        assert!(args
            .iter()
            .any(|s| s.contains("program a.elf verify reset exit")));
    }

    #[test]
    fn args_esp32_uses_interface_and_target() {
        let args = OpenOcdBackend.build_args(&esp_cfg());
        // ESP32 现在也使用 interface + target，不再依赖 board/xxx.cfg
        assert!(args.iter().any(|s| s == "interface/esp_usb_jtag.cfg"));
        assert!(args.iter().any(|s| s == "target/esp32s3.cfg"));
        // ESP 芯片不添加 transport select
        assert!(!args.iter().any(|s| s.contains("transport select")));
        assert!(args
            .iter()
            .any(|s| s.contains("program a.elf verify reset exit")));
    }
}
