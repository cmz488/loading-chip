//! OpenOCD 烧录后端
//!
//! 直接调用 OpenOCD 完成固件烧录，无需外部 GDB Server。
//! 所有芯片统一使用 interface + target 分离模式（兼容主线 OpenOCD 和 Espressif 分支）。

use crate::backend::mappings;
use crate::backend::Backend;
use crate::backend::FlashConfig;

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
        // 命令顺序重要：
        // 1. interface 配置（加载探头驱动）
        // 2. transport select + adapter speed（必须在 target 之前）
        // 3. target 配置（加载芯片 flash 算法）
        // 4. program 命令
        let mut args = vec![
            "-f".to_string(),
            mappings::openocd_interface_cfg(&config.interface).into(),
        ];

        // 传输协议选择（target 配置加载前设置，兼容 CMSIS-DAP 克隆探头）
        if !config.target.starts_with("esp") {
            args.push("-c".into());
            args.push("transport select swd".into());
            // 降低适配器速度提高克隆探头稳定性
            args.push("-c".into());
            args.push("adapter speed 1000".into());
        }

        args.push("-f".into());
        args.push(mappings::openocd_target_cfg(&config.target).into());
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
            backend: crate::backend::FlashBackend::OpenOcd,
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
            backend: crate::backend::FlashBackend::OpenOcd,
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
        // interface 在 target 之前
        let iface_pos = args.iter().position(|s| s == "interface/cmsis-dap.cfg").unwrap();
        let target_pos = args.iter().position(|s| s == "target/stm32f4x.cfg").unwrap();
        assert!(iface_pos < target_pos, "interface must come before target");
        assert!(args.iter().any(|s| s == "transport select swd"));
        assert!(args.iter().any(|s| s == "adapter speed 1000"));
        assert!(args
            .iter()
            .any(|s| s.contains("program a.elf verify reset exit")));
    }

    #[test]
    fn args_esp32_uses_interface_and_target() {
        let args = OpenOcdBackend.build_args(&esp_cfg());
        assert!(args.iter().any(|s| s == "interface/esp_usb_jtag.cfg"));
        assert!(args.iter().any(|s| s == "target/esp32s3.cfg"));
        // ESP 芯片不添加 transport select 和 adapter speed
        assert!(!args.iter().any(|s| s.contains("transport select")));
        assert!(!args.iter().any(|s| s.contains("adapter speed")));
        assert!(args
            .iter()
            .any(|s| s.contains("program a.elf verify reset exit")));
    }
}
