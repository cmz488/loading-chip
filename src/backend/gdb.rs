//! GDB 烧录后端 — arm-none-eabi-gdb
//!
//! 通过 arm-none-eabi-gdb 的 `-batch` 模式连接 GDB Server 完成烧录。
//! 需要外部 GDB Server（如 OpenOCD / pyOCD）已经在指定端口监听。

use crate::backend::Backend;
use crate::FlashConfig;

/// GDB 后端
#[derive(Debug, Clone, Copy, Default)]
pub struct GdbBackend;

impl Backend for GdbBackend {
    fn name(&self) -> &'static str {
        "arm-none-eabi-gdb"
    }

    fn binary(&self) -> &'static str {
        "arm-none-eabi-gdb"
    }

    fn build_args(&self, config: &FlashConfig) -> Vec<String> {
        vec![
            "-batch".to_string(),
            "-ex".to_string(),
            format!("target extended-remote :{}", config.gdb_port),
            "-ex".to_string(),
            "monitor reset halt".to_string(),
            "-ex".to_string(),
            "load".to_string(),
            "-ex".to_string(),
            "monitor reset run".to_string(),
            "-ex".to_string(),
            "quit".to_string(),
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
            backend: crate::FlashBackend::Gdb,
            interface: "swd".into(),
            target: "stm32f4".into(),
            elf_path: "a.elf".into(),
            gdb_port: "3333".into(),
            pyocd_path: String::new(),
            timeout_secs: 0,
            board_config: String::new(),
            board_extra_args: vec![],
            board_id: String::new(),
        }
    }

    #[test]
    fn args_smoke() {
        let args = GdbBackend.build_args(&cfg());
        assert_eq!(args[0], "-batch");
        assert!(args
            .iter()
            .any(|s| s.contains("target extended-remote :3333")));
        assert_eq!(args.last().unwrap(), "a.elf");
    }
}
