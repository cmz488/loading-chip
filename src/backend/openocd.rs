//! OpenOCD 烧录后端
//!
//! 直接调用 OpenOCD 完成固件烧录，无需外部 GDB Server。
//! ESP32 系列使用板级配置，其他芯片使用 interface + target 分离模式。

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
        let is_esp = config.target.starts_with("esp");
        let mut args = Vec::with_capacity(if is_esp { 6 } else { 12 });

        if is_esp {
            args.push("-f".into());
            args.push(mappings::openocd_target_cfg(&config.target).into());
        } else {
            args.push("-f".into());
            args.push(mappings::openocd_interface_cfg(&config.interface).into());
            args.push("-f".into());
            args.push(mappings::openocd_target_cfg(&config.target).into());

            if config.interface == "swd" {
                args.push("-c".into());
                args.push("transport select swd".into());
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
    fn args_esp32_no_interface() {
        let mut c = cfg();
        c.target = "esp32s3".into();
        let args = OpenOcdBackend.build_args(&c);
        assert!(!args.iter().any(|s| s.starts_with("interface/")));
        assert!(args.iter().any(|s| s == "board/esp32s3-builtin.cfg"));
        assert!(args
            .iter()
            .any(|s| s.contains("program a.elf verify reset exit")));
    }
}
