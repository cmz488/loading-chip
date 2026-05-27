//! pyOCD 烧录后端
//!
//! Python 烧录/调试工具，基于 CMSIS-Pack 生态。
//! 通常运行在 Python 虚拟环境中，通过 `pyocd_path` 字段指定可执行文件路径。

use std::path::Path;

use crate::backend::mappings;
use crate::backend::Backend;
use crate::FlashConfig;

/// pyOCD 后端
#[derive(Debug, Clone, Copy, Default)]
pub struct PyOcdBackend;

impl Backend for PyOcdBackend {
    fn name(&self) -> &'static str {
        "pyOCD"
    }

    fn binary(&self) -> &'static str {
        "pyocd"
    }

    fn build_args(&self, config: &FlashConfig) -> Vec<String> {
        vec![
            "flash".into(),
            "--target".into(),
            mappings::pyocd_target(&config.target).into(),
            "--erase".into(),
            "chip".into(),
            config.elf_path.clone(),
        ]
    }

    fn resolve_binary(&self, config: &FlashConfig) -> String {
        // 1. 用户指定的 pyocd 路径（支持 venv）
        if !config.pyocd_path.is_empty() {
            return config.pyocd_path.clone();
        }
        // 2. 环境变量 PYOCD_PATH
        if let Ok(p) = std::env::var("PYOCD_PATH") {
            if !p.is_empty() {
                return p;
            }
        }
        // 3. 自动检测常见 venv 路径
        for candidate in &[".venv/bin/pyocd", "venv/bin/pyocd", "env/bin/pyocd"] {
            if Path::new(candidate).is_file() {
                return candidate.to_string();
            }
        }
        // 4. fallback: PATH 查找
        "pyocd".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FlashConfig {
        FlashConfig {
            backend: crate::FlashBackend::PyOcd,
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
        let args = PyOcdBackend.build_args(&cfg());
        assert_eq!(args[0], "flash");
        assert_eq!(args[1], "--target");
        assert_eq!(args[2], "stm32f407vg");
        assert_eq!(args[3], "--erase");
        assert_eq!(args.last().unwrap(), "a.elf");
    }

    #[test]
    fn binary_resolution() {
        let be = PyOcdBackend;

        // 默认
        let bin = be.resolve_binary(&cfg());
        assert_eq!(bin, "pyocd");

        // 指定 pyocd_path
        let mut c = cfg();
        c.pyocd_path = "/home/user/venv/bin/pyocd".into();
        let bin = be.resolve_binary(&c);
        assert_eq!(bin, "/home/user/venv/bin/pyocd");
    }
}
