//! probe-rs 烧录后端
//!
//! Rust 原生烧录工具，零配置，速度快。直接通过 SWD/JTAG 连接到目标芯片，
//! 无需外部 GDB Server。

use crate::backend::mappings;
use crate::backend::Backend;
use crate::FlashConfig;

/// probe-rs 后端
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeRsBackend;

impl Backend for ProbeRsBackend {
    fn name(&self) -> &'static str {
        "probe-rs"
    }

    fn binary(&self) -> &'static str {
        "probe-rs"
    }

    fn build_args(&self, config: &FlashConfig) -> Vec<String> {
        vec![
            "download".into(),
            "--chip".into(),
            mappings::probe_rs_chip(&config.target).into(),
            config.elf_path.clone(),
        ]
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
            backend: crate::FlashBackend::ProbeRs,
            interface: String::new(),
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
    fn args_smoke() {
        let args = ProbeRsBackend.build_args(&cfg());
        assert_eq!(args[0], "download");
        assert!(args.iter().any(|s| s == "STM32F407VG"));
        assert_eq!(args.last().unwrap(), "a.elf");
    }
}
